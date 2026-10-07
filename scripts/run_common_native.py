#!/usr/bin/env python3
"""Run a fixed common-Client scenario against isolated, sequential vanilla servers.

Results come from native server RCON independently of Voxrig's received cache.
Only disposable run files are modified. An optional runtime parent can isolate
disk I/O; diagnostics are retained under .local/native-client-unification.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import queue
import re
import shutil
import secrets
import signal
import socket
import struct
import subprocess
import threading
import time
import urllib.request
import uuid
import zlib

REPO = Path(__file__).resolve().parents[1]
ROOT = REPO / ".local/native-client-unification"
DOWNLOADS = ROOT / "downloads"
VERSIONS = ("1.16.1", "1.21.11")


def download(version):
    DOWNLOADS.mkdir(parents=True, exist_ok=True)
    source = DOWNLOADS / (version + "-source.json")
    if source.exists():
        record = json.loads(source.read_text())
    else:
        manifest = json.load(urllib.request.urlopen("https://piston-meta.mojang.com/mc/game/version_manifest_v2.json", timeout=30))
        entry = next(value for value in manifest["versions"] if value["id"] == version)
        raw = urllib.request.urlopen(entry["url"], timeout=30).read()
        if hashlib.sha1(raw).hexdigest() != entry["sha1"]:
            raise RuntimeError("metadata SHA-1 mismatch")
        record = {"version": version, "metadata_url": entry["url"], "metadata_sha1": entry["sha1"], "server": json.loads(raw)["downloads"]["server"]}
        source.write_text(json.dumps(record, indent=2) + "\n")
    jar = DOWNLOADS / (version + "-server.jar")
    if not jar.exists():
        with urllib.request.urlopen(record["server"]["url"], timeout=60) as incoming, jar.open("wb") as outgoing:
            while block := incoming.read(1024 * 1024):
                outgoing.write(block)
    if hashlib.sha1(jar.read_bytes()).hexdigest() != record["server"]["sha1"]:
        raise RuntimeError("server SHA-1 mismatch")
    return jar, record


def free_port():
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return listener.getsockname()[1]


class Rcon:
    def __init__(self, port, password):
        self.stream = socket.create_connection(("127.0.0.1", port), timeout=3)
        # Commands can await the isolated CPU-one server's tick/save queue.
        # Wait for the original response; never retry a possibly applied command.
        self.stream.settimeout(10)
        self.serial = 1
        self.send(3, password)
        for _ in range(4):
            identity, kind, _ = self.receive()
            if identity == -1:
                raise RuntimeError("RCON authentication failed")
            if kind == 2 and identity == 1:
                break
        else:
            raise RuntimeError("missing RCON authentication result")

    def send(self, kind, body):
        payload = struct.pack("<ii", self.serial, kind) + body.encode() + b"\0\0"
        self.stream.sendall(struct.pack("<i", len(payload)) + payload)

    def exact(self, count):
        result = b""
        while len(result) < count:
            data = self.stream.recv(count - len(result))
            if not data:
                raise RuntimeError("RCON disconnected")
            result += data
        return result

    def receive(self):
        length, = struct.unpack("<i", self.exact(4))
        if not 10 <= length <= 4 * 1024 * 1024:
            raise RuntimeError("invalid RCON frame length")
        payload = self.exact(length)
        identity, kind = struct.unpack("<ii", payload[:8])
        return identity, kind, payload[8:-2].decode()

    def command(self, text):
        self.serial += 1
        self.send(2, text)
        identity, kind, body = self.receive()
        if identity != self.serial or kind != 0:
            raise RuntimeError("RCON response identity mismatch")
        return body

    def close(self):
        self.stream.close()


def pump(stream, messages, log):
    for line in stream:
        log.write(line)
        log.flush()
        messages.put(line.rstrip())


def until(predicate, timeout=20):
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        result = predicate()
        if result:
            return result
        time.sleep(0.1)
    raise TimeoutError("native result not established")


def matched(response, pattern):
    return response if re.search(pattern, response) else None


class PacketTraceProxy:
    """Byte-for-byte forwarding with read-only compressed-frame diagnostics."""
    def __init__(self, server_port, version, path):
        self.server_port, self.version = server_port, version
        self.listener = socket.socket()
        self.listener.bind(("127.0.0.1", 0))
        self.port = self.listener.getsockname()[1]
        self.listener.listen(4)
        self.listener.settimeout(0.2)
        self.lock, self.stop = threading.Lock(), threading.Event()
        self.frames, self.errors, self.connections, self.workers = [], [], [], []
        self.error_contexts, self.terminal_events = [], []
        self.connection_states, self.terminal_deliveries = [], []
        self.log = path.open("w")
        self.acceptor = threading.Thread(target=self.accept, daemon=True)
        self.acceptor.start()

    @staticmethod
    def read_varint_bytes(stream):
        result = bytearray()
        for _ in range(5):
            value = stream.recv(1)
            if not value:
                if not result:
                    return None
                raise EOFError("partial native frame header")
            result.extend(value)
            if not value[0] & 128:
                return bytes(result)
        raise ValueError("oversized native VarInt")

    @staticmethod
    def varint(data):
        value = 0
        for index, byte in enumerate(data[:5]):
            value |= (byte & 127) << (index * 7)
            if not byte & 128:
                return value, index + 1
        raise ValueError("incomplete native VarInt")

    def accept(self):
        while not self.stop.is_set():
            try:
                client, _ = self.listener.accept()
            except socket.timeout:
                continue
            except OSError:
                break
            upstream = socket.create_connection(("127.0.0.1", self.server_port), timeout=5)
            upstream.settimeout(None)
            client.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
            upstream.setsockopt(socket.IPPROTO_TCP, socket.TCP_NODELAY, 1)
            state = {"compression": None, "phase": "handshake", "connection": len(self.connections) + 1}
            self.connections.append((client, upstream))
            with self.lock:
                self.connection_states.append(state)
            for source, destination, direction in [(client, upstream, "serverbound"), (upstream, client, "clientbound")]:
                worker = threading.Thread(target=self.forward, args=(source, destination, direction, state), daemon=True)
                self.workers.append(worker)
                worker.start()

    def forward(self, source, destination, direction, state):
        header, length, frame, recorded_ordinal = None, None, bytearray(), None
        try:
            while True:
                header, length, frame, recorded_ordinal = None, None, bytearray(), None
                header = self.read_varint_bytes(source)
                if header is None:
                    with self.lock:
                        state["clean_eof_" + direction] = True
                        self.terminal_events.append({"connection":state["connection"],"direction":direction,
                            "phase":state["phase"],"after_frame_ordinal":len(self.frames),"kind":"clean_eof"})
                    break
                length, _ = self.varint(header)
                if not 0 < length <= 8 * 1024 * 1024:
                    raise ValueError("native frame length outside trace bound")
                frame = bytearray()
                while len(frame) < length:
                    part = source.recv(length - len(frame))
                    if not part:
                        raise EOFError("partial native frame body")
                    frame.extend(part)
                with self.lock:
                    decoded = bytes(frame)
                    if state["compression"] is not None:
                        expanded, offset = self.varint(decoded)
                        decoded = zlib.decompress(decoded[offset:]) if expanded else decoded[offset:]
                        if expanded and len(decoded) != expanded:
                            raise ValueError("native expanded frame length differs")
                    packet, offset = self.varint(decoded)
                    body = decoded[offset:]
                    record = {"ordinal": len(self.frames) + 1, "connection": state["connection"], "direction": direction, "phase": state["phase"], "packet_id": packet, "body_length": len(body), "wire_sha256": hashlib.sha256(header + frame).hexdigest(), "body_sha256": hashlib.sha256(body).hexdigest()}
                    if len(body) <= 512 or (self.version == "1.16.1" and state["phase"] == "play" and direction == "clientbound" and packet == 0x25):
                        record["body_hex"] = body.hex()
                    self.frames.append(record)
                    recorded_ordinal = record["ordinal"]
                    self.log.write(json.dumps(record) + "\n")
                    self.log.flush()
                    if direction == "serverbound" and state["phase"] == "handshake":
                        state["phase"] = "login"
                    elif direction == "clientbound" and state["phase"] == "login":
                        if packet == 3:
                            state["compression"], _ = self.varint(body)
                        elif packet == 2:
                            state["phase"] = "play" if self.version == "1.16.1" else "await_login_ack"
                    elif direction == "serverbound" and packet == 3:
                        if state["phase"] == "await_login_ack":
                            state["phase"] = "configuration"
                        elif state["phase"] == "configuration":
                            state["phase"] = "play"
                # The exact original framed bytes are forwarded, not the
                # decoded body; no injection, re-encoding or native prediction.
                destination.sendall(header + frame)
        except (OSError, EOFError) as error:
            if not self.stop.is_set() and not isinstance(error, ConnectionResetError):
                with self.lock:
                    context = {"error":str(error),"connection":state["connection"],
                        "direction":direction,"phase":state["phase"],"after_frame_ordinal":len(self.frames),
                        "recorded_frame_ordinal":recorded_ordinal,"disconnect_requested":state.get("disconnect_requested",False),
                        "expected_body_length":length,"received_body_length":len(frame),
                        "partial_wire_sha256":hashlib.sha256((header or b"")+frame).hexdigest(),
                        "opposite_clean_eof":state.get("clean_eof_"+("clientbound" if direction=="serverbound" else "serverbound"),False)}
                    if (direction == "clientbound" and isinstance(error,BrokenPipeError)
                            and state.get("disconnect_requested",False) and recorded_ordinal is not None):
                        context.update(source_frame_complete=True, delivered=False,
                            authority_limits="Original complete native frame after this connection's explicitly requested Client disconnect; no Client receipt or complete delivery is inferred.")
                        self.terminal_deliveries.append(context)
                    else:
                        self.errors.append(str(error))
                        self.error_contexts.append(context)
        except BaseException as error:
            with self.lock:
                self.errors.append(str(error))
        finally:
            try:
                destination.shutdown(socket.SHUT_WR)
            except OSError:
                pass

    def mark(self):
        with self.lock:
            return len(self.frames)

    def expect_disconnect(self, connection=None):
        """Scope terminal delivery diagnostics to the actual requested connection."""
        with self.lock:
            if not self.connection_states:
                raise RuntimeError("disconnect requested without a traced connection")
            state = self.connection_states[-1] if connection is None else next((s for s in self.connection_states if s["connection"] == connection), None)
            if state is None:
                raise RuntimeError("disconnect requested for an unknown traced connection")
            state["disconnect_requested"] = True
            self.terminal_events.append({"connection":state["connection"],"direction":"caller",
                "phase":state["phase"],"after_frame_ordinal":len(self.frames),"kind":"disconnect_requested"})

    def since(self, boundary):
        with self.lock:
            return [dict(frame) for frame in self.frames[boundary:]]

    def close(self):
        self.stop.set()
        self.listener.close()
        for pair in self.connections:
            for connection in pair:
                try:
                    connection.shutdown(socket.SHUT_RDWR)
                except OSError:
                    pass
                connection.close()
        self.acceptor.join(timeout=2)
        for worker in self.workers:
            worker.join(timeout=2)
        self.log.close()


def legacy_codec_dimensions(raw):
    """Independently retain named dimension-list compound spans from original JOIN NBT."""
    offset = 0
    def take(size):
        nonlocal offset
        if size < 0 or offset + size > len(raw):
            raise ValueError("truncated native registry NBT")
        value = raw[offset:offset + size]; offset += size
        return value
    def string():
        return take(struct.unpack(">H", take(2))[0])
    def value(kind, depth=0):
        if depth > 64: raise ValueError("native registry NBT nesting")
        start = offset
        if kind in (1, 2, 3, 4, 5, 6):
            result = take({1:1,2:2,3:4,4:8,5:4,6:8}[kind])
        elif kind == 8:
            result = string()
        elif kind in (7, 11, 12):
            count = struct.unpack(">i", take(4))[0]
            result = take(count * {7:1,11:4,12:8}[kind])
        elif kind == 9:
            child = take(1)[0]; count = struct.unpack(">i", take(4))[0]
            if not 0 <= count <= len(raw): raise ValueError("native registry list count")
            result = [value(child, depth + 1) for _ in range(count)]
        elif kind == 10:
            result = {}
            while (child := take(1)[0]) != 0:
                name = string()
                result[name] = value(child, depth + 1)
        else:
            raise ValueError("invalid native registry NBT type")
        return (kind, result, start, offset)
    if take(1) != b"\x0a": raise ValueError("native registry root must be named compound")
    string(); root = value(10)
    if offset != len(raw): raise ValueError("trailing native registry NBT")
    dimensions = root[1].get(b"dimension")
    if dimensions is None: return None
    if dimensions[0] != 9: raise ValueError("native dimension declaration must be list")
    entries = []
    for kind, compound, start, end in dimensions[1]:
        if kind != 10 or compound[b"name"][0] != 8: raise ValueError("native dimension entry")
        entries.append({"name":compound[b"name"][1].decode("ascii"), "data":list(b"\x0a" + raw[start:end])})
    return entries


def verify_received_registries(snapshot, player, trace, version):
    """Check public received data against original forwarded payload hashes/ordinals."""
    state = snapshot["received"]
    if not state["complete"] or state["session"] != player["session"] or state["receive_sequence"] < player["receive_sequence"]:
        raise RuntimeError("registry observation is not bound to the captured player connection")
    frames = trace.since(0)
    connection = max(frame["connection"] for frame in frames)
    received = [frame for frame in frames if frame["connection"] == connection and frame["direction"] == "clientbound" and frame["phase"] in ("configuration", "play")]
    verified = []
    def varint(value):
        out = bytearray()
        while value >= 128:
            out.append((value & 127) | 128); value >>= 7
        out.append(value)
        return bytes(out)
    def string(value):
        data = value.encode("utf-8")
        return varint(len(data)) + data
    def packet(source, expected, payload=None):
        if source["kind"] != "received": raise RuntimeError("registry provenance is not received")
        frame = received[source["sequence"] - 1]
        if frame["packet_id"] != expected: raise RuntimeError("registry source ordinal refers to another original packet")
        if payload is not None and (len(payload) != frame["body_length"] or hashlib.sha256(payload).hexdigest() != frame["body_sha256"]):
            raise RuntimeError("received registry payload differs from original native frame")
        verified.append({key: frame[key] for key in ("ordinal", "connection", "phase", "packet_id", "body_length", "body_sha256")})
        return frame
    for name, observation in (state["registries"].items() if version != "1.16.1" else []):
        entries = observation["value"]
        payload = string(name) + varint(len(entries))
        payload += b"".join(string(entry["name"]) + b"\x01" + bytes(entry["data"]) for entry in entries)
        packet(observation["source"], 7, payload)
    if state["tags"] is None or state["tag_packet"] is None:
        raise RuntimeError("original native tag declaration was not retained")
    tags = state["tag_packet"]
    if tags["source"] != state["tags"]["source"]: raise RuntimeError("parsed/raw tag provenance differs")
    expected_tag_packet = 0x5b if version == "1.16.1" else (0x0d if received[tags["source"]["sequence"] - 1]["phase"] == "configuration" else 0x84)
    packet(tags["source"], expected_tag_packet, bytes(tags["value"]))
    if version == "1.16.1":
        if set(state["registries"]) - {"minecraft:dimension_type"} or snapshot["unbreaking"] is not None or state["legacy_codec"] is None:
            raise RuntimeError("legacy registry observation fabricated a modern entry list")
        codec = state["legacy_codec"]
        frame = packet(codec["source"], 0x25)
        data = bytes.fromhex(frame["body_hex"])
        count, length = PacketTraceProxy.varint(data[6:]); offset = 6 + length
        for _ in range(count):
            size, length = PacketTraceProxy.varint(data[offset:]); offset += length + size
        raw = bytes(codec["value"])
        if data[offset:offset + len(raw)] != raw or raw[:1] != b"\x0a":
            raise RuntimeError("legacy codec differs from original join field")
        dimensions = legacy_codec_dimensions(raw)
        declaration = state["registries"].get("minecraft:dimension_type")
        if dimensions is None:
            if declaration is not None: raise RuntimeError("legacy dimension declaration fabricated")
        elif declaration is None or declaration["source"] != codec["source"] or declaration["value"] != dimensions:
            raise RuntimeError("legacy dimension names/order/compound bytes differ from original join codec")
    else:
        if state["legacy_codec"] is not None: raise RuntimeError("modern observation fabricated legacy NBT")
        binding = snapshot["unbreaking"]
        if binding["id"]["stamp"] != state["stamp"] or binding["id"]["registry"] != "minecraft:enchantment" or binding["entry"]["name"] != "minecraft:unbreaking":
            raise RuntimeError("enchantment name does not belong to this live registry")
        patch = player["inventory"]["slots"][9]["value"]["item"]["data"]["patch"]
        component = next(c for c in patch["added"] if c["definition"]["name"] == "minecraft:enchantments")
        raw = bytes(component["bytes"])
        count, offset = PacketTraceProxy.varint(raw)
        identity, size = PacketTraceProxy.varint(raw[offset:]); offset += size
        level, size = PacketTraceProxy.varint(raw[offset:]); offset += size
        if (count, identity, level, offset) != (1, binding["id"]["value"], 2, len(raw)):
            raise RuntimeError("actual enchantment component does not reference the live unbreaking ID/level")
    return {"stamp": state["stamp"], "session": state["session"], "receive_sequence": state["receive_sequence"],
        "registry_count": len(state["registries"]), "tag_registry_count": len(state["tags"]["value"]),
        "unbreaking": snapshot["unbreaking"], "verified_original_frames": verified,
        "observation_sha256": hashlib.sha256(json.dumps(snapshot, sort_keys=True, separators=(",", ":")).encode()).hexdigest(),
        "authority_limits": "Received configuration entries/raw tags and legacy join codec match original forwarded fields/ordinals. One actual modern enchantment reference resolves against this same connection. General component meaning, prototypes, effective properties and data-bearing actions remain separate work."}


def outer_snbt_compounds(response):
    """Keep entire inventory entries, including nested default item tags."""
    result, depth, start, quoted, escaped = [], 0, None, False, False
    for index, char in enumerate(response):
        if quoted:
            if escaped:
                escaped = False
            elif char == "\\":
                escaped = True
            elif char == '"':
                quoted = False
        elif char == '"':
            quoted = True
        elif char == "{":
            if depth == 0:
                start = index
            depth += 1
        elif char == "}":
            depth -= 1
            if depth < 0:
                raise RuntimeError("unbalanced native inventory SNBT")
            if depth == 0:
                result.append(response[start:index + 1])
    if depth or quoted:
        raise RuntimeError("incomplete native inventory SNBT")
    return result


def stage(probe, messages, name, records, timeout=30, poll=None):
    if name not in ("ready", "mining_ready", "placement_ready", "swap_ready", "container_ready", "workflow_ready", "a2_ready", "a4_ready", "a4_captured", "a5_ui_ready", "a5_furnace_ready", "a5_vehicle_ready", "b3_terrain_ready", "b5_revocation_ready", "b4_recipe_ready", "b4_ghost_ready"):
        probe.stdin.write(name + "\n")
        probe.stdin.flush()
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if poll:
            poll()
        if probe.poll() is not None and messages.empty():
            raise RuntimeError("probe stopped before " + name)
        try:
            line = messages.get(timeout=0.1 if poll else 0.25)
        except queue.Empty:
            continue
        record = json.loads(line)
        records.append(record)
        aliases = {"disconnect": "disconnected", "mining_disconnect": "mining_disconnected", "placement_disconnect": "placement_disconnected", "swap_disconnect": "swap_disconnected", "container_disconnect": "container_disconnected"}
        if record["stage"] == name or record["stage"] == aliases.get(name):
            print("native", name, "received", flush=True)
            return record
    raise TimeoutError("probe stage timed out: " + name)


def run_dry_terrain(version, env, rcon, trace, report, probe_log, stderr_log, flight=False, landing=False):
    """Walk a dry slab/stair route, then open/close storage on the same Client.
    RCON only reads after baseline. Actual native position and original packet
    fields are separate from Client predictions and local close receipts.
    """
    results = report['native_results']['creative_landing' if landing else ('creative_flight' if flight else 'dry_terrain')] = {}
    for mode in (('creative',) if flight else ('survival', 'creative')):
        until(lambda: matched(rcon.command('execute unless entity @a[name=UnifiedProbe]'), 'Test passed'))
        result = results[mode] = {'fixture': {}, 'records': []}
        for command in [
            'fill -1 65 -1 2 70 8 minecraft:air',
            'fill -1 65 1 1 65 1 minecraft:stone_slab[type=bottom,waterlogged=false]',
            'fill -1 65 2 1 65 3 minecraft:stone',
            'fill -1 66 3 1 66 3 minecraft:oak_stairs[facing=south,half=bottom,shape=straight,waterlogged=false]',
            'fill -1 65 4 1 66 8 minecraft:stone',
            'setblock 2 67 5 minecraft:chest[facing=west,type=single,waterlogged=false]',
        ]:
            response = rcon.command(command)
            result['fixture'][command] = response
            if any(text in response for text in ('Incorrect argument', 'Unknown or incomplete', 'not loaded')):
                raise RuntimeError('dry terrain fixture rejected: '+response)
        probe = subprocess.Popen([str(REPO/'target/debug/examples/common_native_probe')], cwd=REPO,
            env=dict(env, VOXRIG_NATIVE_SCENARIO='dry-terrain', VOXRIG_NATIVE_MODE=mode),
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=stderr_log, text=True, bufsize=1)
        messages = queue.Queue()
        reader = threading.Thread(target=pump, args=(probe.stdout, messages, probe_log), daemon=True)
        reader.start()
        try:
            stage(probe, messages, 'b3_terrain_ready', result['records'])
            for command in ['gamemode '+mode+' UnifiedProbe', 'tp UnifiedProbe 0.5 65 0.5 0 0', 'clear UnifiedProbe']:
                result['fixture'][command] = rcon.command(command)
            baseline = stage(probe, messages, 'b3_terrain_baseline', result['records'])['value']
            result['position_before'] = rcon.command('data get entity UnifiedProbe Pos')
            boundary = trace.mark()
            moved = stage(probe, messages, 'b3_terrain_move', result['records'])['value']
            frames = baseline['preview']['frames']
            expected = frames[-1]['position']
            def endpoint():
                raw = rcon.command('data get entity UnifiedProbe Pos')
                actual = [float(v) for v in re.findall(r'(-?\d+(?:\.\d+)?(?:[Ee][+-]?\d+)?)d', raw)]
                return raw if len(actual)==3 and all(abs(a-b)<1e-7 for a,b in zip(actual,expected)) else None
            result['position_after_move'] = until(endpoint)
            if len(frames)!=44 or expected[1]!=67.0 or expected[2]<4.0:
                raise RuntimeError('dry terrain route did not reach raised platform')
            position_id = 0x13 if version=='1.16.1' else 0x1e
            motion_frames = until(lambda: (current if len([f for f in current
                if f['direction']=='serverbound' and f['phase']=='play' and f['packet_id']==position_id])>=44 else None)
                if (current := trace.since(boundary)) else None)
            positions = [f for f in motion_frames if f['direction']=='serverbound'
                and f['phase']=='play' and f['packet_id']==position_id]
            def fields(frame):
                raw = bytes.fromhex(frame['body_hex'])
                if len(raw)!=33:raise RuntimeError('dry terrain position packet length differs')
                return (*struct.unpack('>dddff', raw[:32]), raw[32])
            expected_fields = [(*f['position'], 0.0, 0.0, int(f['on_ground'])
                | (int(f['horizontal_collision'])<<1 if version!='1.16.1' else 0)) for f in frames]
            actual_fields = [fields(f) for f in positions]
            starts = [i for i in range(len(positions)-43) if actual_fields[i:i+44]==expected_fields]
            if len(starts)!=1 or moved['record']['dispatched_ticks']!=44:
                raise RuntimeError('dry terrain trace lacks one complete fixed 44-frame path')
            start = starts[0]
            if version=='1.16.1':
                initial = (*baseline['preview']['initial_frame']['position'],0.0,0.0)
                if any(f[:5]!=initial or f[5] not in (0,1) for f in actual_fields[:start]) or any(
                    f!=expected_fields[-1] for f in actual_fields[start+44:]):
                    raise RuntimeError('legacy native idle frames changed initial/terminal position')
            elif start!=0 or len(positions)!=44:
                raise RuntimeError('modern dry terrain has extra full-position frames')
            # The legacy native ground loop can emit idle position heartbeats
            # before/after a finite run. Keep and inspect them separately; they
            # are not counted as that run's dispatched controls or as receipts.
            result['finite_motion_trace'] = {'matching_frames':44,
                'first_ordinal':positions[start]['ordinal'],'last_ordinal':positions[start+43]['ordinal'],
                'idle_before':positions[:start],'idle_after':positions[start+44:]}
            for actual, predicted in zip(positions[start:start+44], frames):
                body = bytes.fromhex(actual['body_hex'])
                values = struct.unpack('>dddff', body[:32])
                if list(values[:3])!=predicted['position'] or values[3]!=0.0 or values[4]!=0.0:
                    raise RuntimeError('dry terrain original frame differs from declared predicted position/rotation')
                flags = int(predicted['on_ground']) | (int(predicted['horizontal_collision'])<<1 if version!='1.16.1' else 0)
                if len(body)!=33 or body[32]!=flags:
                    raise RuntimeError('dry terrain original ground flag differs from prediction')
            if version!='1.16.1':
                inputs = [f for f in motion_frames if f['direction']=='serverbound'
                    and f['phase']=='play' and f['packet_id']==0x2a]
                if [f['body_hex'] for f in inputs]!=['01']*20+['00']*24:
                    raise RuntimeError('dry terrain native input differs from fixed common controls')
            storage_boundary = trace.mark()
            stored = stage(probe, messages, 'b3_terrain_storage', result['records'])['value']
            result['position_after_storage'] = until(endpoint)
            result['chest_contents'] = rcon.command('data get block 2 67 5 Items')
            if outer_snbt_compounds(result['chest_contents']):
                raise RuntimeError('dry terrain opening altered native chest contents')
            session = baseline['player']['session']
            if any(p['session']!=session for p in (moved['player'], stored['player'])):
                raise RuntimeError('dry terrain workflow changed Client/session')
            opening = stored['opening']['observed_screen']['id']
            if not stored['close']['dispatched'] or stored['close']['id']['screen']!=opening:
                raise RuntimeError('dry terrain close did not refer to received opening')
            storage_frames = until(lambda: (current if any(f['direction']=='serverbound' and f['phase']=='play'
                and f['packet_id']==(0x0a if version=='1.16.1' else 0x12) for f in current) else None)
                if (current := trace.since(storage_boundary)) else None)
            for packet in (0x2d,0x0a) if version=='1.16.1' else (0x3f,0x12):
                if len([f for f in storage_frames if f['direction']=='serverbound' and f['phase']=='play' and f['packet_id']==packet])!=1:
                    raise RuntimeError('dry terrain storage did not dispatch activation/close exactly once')
            connection = positions[0]['connection']
            peers = [f for f in trace.since(0) if f['connection']==connection
                and f['direction']=='clientbound' and f['phase'] in ('configuration','play')]
            original_open = peers[opening['opened_sequence']-1]
            if original_open['packet_id']!=(0x2e if version=='1.16.1' else 0x39):
                raise RuntimeError('dry terrain received opening ordinal refers to another native packet')
            raw = bytes.fromhex(original_open['body_hex'])
            window, offset = PacketTraceProxy.varint(raw)
            menu, _ = PacketTraceProxy.varint(raw[offset:])
            if window!=opening['window'] or menu!=2 or stored['opening']['observed_screen']['menu_name']!='minecraft:generic_9x3':
                raise RuntimeError('dry terrain received window/menu differs from original native OPEN')
            result['opening_source_proof'] = {k:original_open[k] for k in ('connection','ordinal','packet_id','body_sha256')}
            result['motion_frames'] = motion_frames
            result['storage_frames'] = storage_frames
            result['authority_limits'] = 'Same common consumer and one Client per mode. Actual dry slab/stair properties, 44 bounded predicted ticks, independent native final position and unchanged session through actual chest OPEN and once close. Survival captured-scene prediction also matches. Sent positions and local close are not server acknowledgements. No waterlogged, body-intersecting seed, tool/effect/posture, vehicle, flight-to-standing or arbitrary terrain support is claimed.'
            if flight:
                flight_boundary = trace.mark()
                flown = stage(probe, messages, 'b3_flight_steps', result['records'])['value']
                commands = flown['commands']
                if len(commands)!=4 or commands[0]['command']!={'kind':'set_flying','flying':True}:
                    raise RuntimeError('flight enable record differs from requested command')
                if [r['attempt'] for r in commands]!=list(range(commands[0]['attempt'],commands[0]['attempt']+4)):
                    raise RuntimeError('flight attempt identities do not increase')
                if any(not r['dispatched'] or r['stage']!='submitted' or r['requires_inspection'] for r in commands):
                    raise RuntimeError('flight command dispatch remained unresolved')
                final = commands[-1]['command']['position']
                def airborne_endpoint():
                    raw = rcon.command('data get entity UnifiedProbe Pos')
                    actual = [float(v) for v in re.findall(r'(-?\d+(?:\.\d+)?(?:[Ee][+-]?\d+)?)d',raw)]
                    return raw if len(actual)==3 and all(abs(a-b)<1e-7 for a,b in zip(actual,final)) else None
                result['flight_native_position'] = until(airborne_endpoint)
                result['flight_native_abilities'] = rcon.command('data get entity UnifiedProbe abilities')
                if not re.search(r'flying:\s*1b',result['flight_native_abilities']):
                    raise RuntimeError('server did not apply requested flight flag')
                flight_frames = until(lambda: (current if len([f for f in current if f['direction']=='serverbound'
                    and f['phase']=='play' and f['packet_id']==position_id])>=3 else None)
                    if (current := trace.since(flight_boundary)) else None)
                sent = [f for f in flight_frames if f['direction']=='serverbound' and f['phase']=='play']
                ability_id = 0x1a if version=='1.16.1' else 0x27
                # Idle ground heartbeats can precede the owner acquiring flight.
                ability_indexes = [i for i,f in enumerate(sent) if f['packet_id']==ability_id]
                if len(ability_indexes)!=1 or sent[ability_indexes[0]]['body_hex']!='02':
                    raise RuntimeError('native flight enable was not dispatched once')
                poses = [f for f in sent[ability_indexes[0]+1:] if f['packet_id']==position_id]
                if len(poses)!=3:
                    raise RuntimeError('native flight steps replayed or missing')
                for frame,record in zip(poses,commands[1:]):
                    if fields(frame)!=(*record['command']['position'],0.,0.,0):
                        raise RuntimeError('native flight frame differs from retained command')
                if flown['player']['session']!=session or flown['player']['received_pose']!=flown['initial']['received_pose']:
                    raise RuntimeError('flight forged receipt or changed connection')
                result['flight_commands'] = commands
                result['flight_frames'] = flight_frames
                if landing:
                    landing_boundary=trace.mark()
                    landed=stage(probe,messages,'b3_flight_land',result['records'])['value']
                    record=landed['record'];stop=record['landing'];frames=stop['motion']['preview']['frames']
                    if record['command']!={'kind':'land'} or not record['dispatched'] or not stop['disable_dispatched'] or not stop['neutral_dispatched']:
                        raise RuntimeError('landing did not finish owned disable/neutral/ground writes')
                    if stop['declared_controller_velocity']!=[0.,0.,0.] or len(frames)!=2 or stop['motion']['dispatched_ticks']!=2:
                        raise RuntimeError('landing declared seed or bounded tick counts differ')
                    expected=frames[-1]['position']
                    result['native_landing_position']=until(endpoint)
                    result['native_landing_abilities']=until(lambda: raw if re.search(r'flying:\s*0b',(raw:=rcon.command('data get entity UnifiedProbe abilities'))) else None)
                    def landing_snapshot():
                        current=trace.since(landing_boundary)
                        poses=[f for f in current if f['direction']=='serverbound' and f['phase']=='play' and f['packet_id']==position_id]
                        return current if len(poses)>=3 else None
                    landing_frames=until(landing_snapshot)
                    sent=[f for f in landing_frames if f['direction']=='serverbound' and f['phase']=='play']
                    disables=[f for f in sent if f['packet_id']==ability_id]
                    if len(disables)!=1 or disables[0]['body_hex']!='00':raise RuntimeError('landing disable repeated or absent')
                    after_disable=[f for f in sent if f['ordinal']>disables[0]['ordinal'] and f['packet_id']==position_id]
                    if len(after_disable)<2 or [fields(f) for f in after_disable[:2]]!=[(*f['position'],0.,0.,int(f['on_ground'])) for f in frames]:
                        raise RuntimeError('landing original ground frames differ from declared stop model')
                    if version=='1.16.1' and any(fields(f)!=fields(after_disable[1]) for f in after_disable[2:]):
                        raise RuntimeError('legacy idle changed landing endpoint')
                    neutrals=[f['body_hex'] for f in sent if f['packet_id']==(0x1d if version=='1.16.1' else 0x2a)]
                    if neutrals!=(['00'*9] if version=='1.16.1' else ['00']*3):raise RuntimeError('landing native neutral inputs differ')
                    if landed['player']['session']!=session or landed['player']['received_pose']!=flown['player']['received_pose']:
                        raise RuntimeError('landing invented receipt or changed session')
                    result['landing_frames']=landing_frames
                    ground_boundary=trace.mark()
                    moved_again=stage(probe,messages,'b3_landing_ground_move',result['records'])['value']
                    predicted=landed['ground_preview']['frames'];expected=predicted[-1]['position']
                    result['native_post_landing_move_position']=until(endpoint)
                    if moved_again['record']['preview']['frames']!=predicted or moved_again['record']['dispatched_ticks']!=27:
                        raise RuntimeError('post-landing ground run differs from prediction')
                    result['post_landing_move_frames']=trace.since(ground_boundary)
                    second_storage_boundary=trace.mark()
                    again=stage(probe,messages,'b3_terrain_storage',result['records'])['value']
                    if again['player']['session']!=session or not again['close']['dispatched']:
                        raise RuntimeError('landing ground/storage continuation changed connection or left close unresolved')
                    second_open=again['opening']['observed_screen']['id']
                    peers=[f for f in trace.since(0) if f['connection']==connection and f['direction']=='clientbound' and f['phase'] in ('configuration','play')]
                    actual_open=peers[second_open['opened_sequence']-1]
                    if actual_open['packet_id']!=(0x2e if version=='1.16.1' else 0x39):raise RuntimeError('post-landing opening source is not native OPEN')
                    raw=bytes.fromhex(actual_open['body_hex']);window,offset=PacketTraceProxy.varint(raw);menu,_=PacketTraceProxy.varint(raw[offset:])
                    if window!=second_open['window'] or menu!=2 or second_open==opening:raise RuntimeError('post-landing opening did not establish a fresh native chest screen')
                    def second_storage_snapshot():
                        current=trace.since(second_storage_boundary)
                        return current if any(f['direction']=='serverbound' and f['phase']=='play'
                            and f['packet_id']==(0x0a if version=='1.16.1' else 0x12) for f in current) else None
                    second_frames=until(second_storage_snapshot)
                    for packet in (0x2d,0x0a) if version=='1.16.1' else (0x3f,0x12):
                        if len([f for f in second_frames if f['direction']=='serverbound' and f['phase']=='play' and f['packet_id']==packet])!=1:
                            raise RuntimeError('post-landing activation/close missing or replayed')
                    result['post_landing_storage_frames']=second_frames
                    result['post_landing_open_source']={k:actual_open[k] for k in ('connection','ordinal','packet_id','body_sha256')}
                    result['post_landing_chest_contents']=rcon.command('data get block 2 67 5 Items')
                    if outer_snbt_compounds(result['post_landing_chest_contents']):raise RuntimeError('post-landing storage altered native chest')
                    result['authority_limits']+=' Explicit landing uses one fully submitted return to known dry floor, one disable, one neutral release and two released declared ground ticks; then 27 ground ticks and another real chest OPEN/close on the same Client. Received pose/velocity/abilities remain receipts; zero stop velocity is explicitly local controller intent, never server rest ACK.'
                else:
                    disable_boundary = trace.mark()
                    disabled = stage(probe, messages, 'b3_flight_disable', result['records'])['value']
                    result['disable_frames'] = until(lambda: (current if any(f['direction']=='serverbound'
                        and f['phase']=='play' and f['packet_id']==ability_id for f in current) else None)
                        if (current := trace.since(disable_boundary)) else None)
                    flags = [f['body_hex'] for f in result['disable_frames'] if f['direction']=='serverbound'
                        and f['phase']=='play' and f['packet_id']==ability_id]
                    if flags!=['00'] or not disabled['record']['dispatched']:
                        raise RuntimeError('native flight disable was not dispatched once')
                    result['flight_native_abilities_after_disable'] = until(lambda: raw if re.search(r'flying:\s*0b', (raw := rcon.command('data get entity UnifiedProbe abilities'))) else None)
                    result['authority_limits'] += ' Ground -> owned flight enable -> three once-only bounded submitted steps -> disable was validated; explicit landing/standing continuation remains required. Received pose and received velocity are never rewritten as flight endpoints.'
            trace.expect_disconnect()
            stage(probe, messages, 'b3_terrain_disconnect', result['records'])
            probe.wait(timeout=10)
            reader.join(timeout=2)
            if probe.returncode!=0 or reader.is_alive():
                raise RuntimeError('dry terrain probe failed at disconnect')
            result['result'] = 'passed'
        finally:
            if probe.poll() is None:
                probe.terminate()
                try:
                    probe.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    probe.kill(); probe.wait(timeout=5)


def run_vehicle(version, env, rcon, trace, report, probe_log, stderr_log, controlling=False):
    """One common mount -> owned request -> actual absence -> explicit neutral.
    Native RootVehicle.Attach provides an independent original player-state check.
    After baseline RCON is read-only. Player entities cannot be assumed to appear
    in vehicle saveAsPassenger NBT, so that serialization is not used as proof.
    """
    results = report['native_results']['vehicle_control' if controlling else 'vehicle'] = {}
    for mode in ('survival', 'creative'):
        until(lambda: matched(rcon.command('execute unless entity @a[name=UnifiedProbe]'), 'Test passed'))
        result = results[mode] = {'fixture': {}, 'records': []}
        for command in ['kill @e[tag=UnifiedMount]', 'setblock 0 65 1 minecraft:air']:
            result['fixture'][command] = rcon.command(command)
        if controlling:
            for command in ['fill 0 64 2 0 64 14 minecraft:stone','fill 0 65 2 0 65 14 minecraft:rail[shape=north_south]','setblock 3 65 3 minecraft:chest[facing=north,type=single,waterlogged=false]']:
                result['fixture'][command]=rcon.command(command)
        probe = subprocess.Popen([str(REPO/'target/debug/examples/common_native_probe')], cwd=REPO,
            env=dict(env, VOXRIG_NATIVE_SCENARIO='vehicle-control' if controlling else 'vehicle', VOXRIG_NATIVE_MODE=mode),
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=stderr_log, text=True, bufsize=1)
        messages = queue.Queue()
        reader = threading.Thread(target=pump, args=(probe.stdout, messages, probe_log), daemon=True)
        reader.start()
        try:
            stage(probe, messages, 'a5_vehicle_ready', result['records'])
            # The previous mode may save a walked position next to this track.
            # Admit the player at the initial fixture position before creating
            # the new original cart, so login collision cannot move its spawn.
            # All fixture writes still precede the operation baseline.
            for command in ['gamemode '+mode+' UnifiedProbe', 'tp UnifiedProbe 0.5 65 0.5 0 0', 'clear UnifiedProbe',
                            'summon minecraft:minecart 0.5 65.1 2.5 {Tags:["UnifiedMount"],NoGravity:1b,Invulnerable:1b}']:
                result['fixture'][command] = rcon.command(command)
            baseline = stage(probe, messages, 'a5_vehicle_baseline', result['records'])['value']
            cart = baseline['cart']
            result['cart_uuid'] = rcon.command('data get entity @e[tag=UnifiedMount,limit=1] UUID')
            def native_uuid(response):
                match = re.search(r'\[I;\s*([^\]]+)\]', response)
                if not match: raise RuntimeError('missing native UUID array: '+response)
                return b''.join(struct.pack('>i', int(s.strip())) for s in match[1].split(','))
            expected_uuid = bytes(cart['uuid'])
            if native_uuid(result['cart_uuid']) != expected_uuid:
                raise RuntimeError('received vehicle UUID differs from original native entity')
            if controlling:
                approach=baseline['approach'];expected=approach['preview']['frames'][-1]['position']
                result['native_approach_position']=rcon.command('data get entity UnifiedProbe Pos')
                def pos(response):
                    return [float(s.strip().rstrip('d')) for s in re.search(r'\[([^\]]+)\]',response).group(1).split(',')]
                actual=pos(result['native_approach_position'])
                if any(abs(a-b)>1e-6 for a,b in zip(expected,actual)):raise RuntimeError('native approach differs from finite ground model')
            boundary = trace.mark()
            mounted = stage(probe, messages, 'a5_vehicle_mount', result['records'])['value']
            result['native_mounted'] = rcon.command('data get entity UnifiedProbe RootVehicle.Attach')
            if native_uuid(result['native_mounted']) != expected_uuid:
                raise RuntimeError('original native player is not riding the received vehicle')
            if controlling:
                result['native_cart_before']=rcon.command('data get entity @e[tag=UnifiedMount,limit=1] Pos')
                controlled=stage(probe,messages,'b6_vehicle_control',result['records'])['value']
                result['native_cart_after']=rcon.command('data get entity @e[tag=UnifiedMount,limit=1] Pos')
                before,after=pos(result['native_cart_before']),pos(result['native_cart_after'])
                if abs(after[0]-before[0])>1e-6 or after[2]<=before[2]+0.05:raise RuntimeError('original minecart did not respond to native forward input')
                if controlled['control']['stage']!='submitted' or controlled['control']['dispatched_ticks']!=14:raise RuntimeError('finite input run missing final submission')
            requested = stage(probe, messages, 'a5_vehicle_request' , result['records'])['value']
            observed = stage(probe, messages, 'a5_vehicle_observed', result['records'])['value']
            result['native_player_present'] = matched(rcon.command(
                'execute if entity @a[name=UnifiedProbe]'), 'Test passed')
            if not result['native_player_present']:
                raise RuntimeError('original native player disappeared during dismount')
            result['native_unmounted'] = until(lambda: matched(rcon.command(
                'execute unless entity @a[name=UnifiedProbe,nbt={RootVehicle:{}}]'), 'Test passed'))
            completed = stage(probe, messages, 'a5_vehicle_complete', result['records'])['value']
            entity_id = 0x0e if version == '1.16.1' else 0x19
            input_id = 0x1d if version == '1.16.1' else 0x2a
            # A complete Client write can precede the transparent proxy thread's
            # append. Wait for that original frame, without synthesizing an ACK.
            frames = until(lambda: (current if len([f for f in current
                if f['direction']=='serverbound' and f['phase']=='play' and f['packet_id']==input_id])>=(16 if controlling else 2) else None)
                if (current := trace.since(boundary)) else None)
            interact = [f for f in frames if f['direction']=='serverbound' and f['phase']=='play' and f['packet_id']==entity_id]
            inputs = [f for f in frames if f['direction']=='serverbound' and f['phase']=='play' and f['packet_id']==input_id]
            if len(interact)!=1 or len(inputs)!=(16 if controlling else 2):
                raise RuntimeError('vehicle workflow needs exactly one INTERACT and two input frames')
            raw = bytes.fromhex(interact[0]['body_hex']); target, offset = PacketTraceProxy.varint(raw)
            if target != cart['id']['native_id'] or raw[offset:] != b'\x00\x00\x00':
                raise RuntimeError('vehicle interaction fields differ from the public intent')
            expected = ['000000000000000002','000000000000000000'] if version=='1.16.1' else ['20','00']
            if controlling:
                forward='000000003f80000000' if version=='1.16.1' else '01'
                neutral='000000000000000000' if version=='1.16.1' else '00'
                expected=[forward]*12+[neutral]*2+expected
            if [f['body_hex'] for f in inputs] != expected:
                raise RuntimeError('native dismount request/neutral differ from original packet codecs')
            connection = interact[0]['connection']
            peers = [f for f in trace.since(0) if f['connection']==connection and f['direction']=='clientbound' and f['phase'] in ('configuration','play')]
            source_proof = []
            def verify(observation, wanted_mounted):
                if observation['session'] != baseline['player']['session']:
                    raise RuntimeError('vehicle workflow changed connection/world')
                relation, passengers = observation['relation'], observation['passengers']
                if relation['source'] != passengers['source'] or relation['source']['kind']!='received':
                    raise RuntimeError('vehicle relationship lacks one original passenger source')
                source = peers[relation['source']['sequence']-1]
                if source['packet_id'] != (0x4b if version=='1.16.1' else 0x69):
                    raise RuntimeError('passenger receive ordinal points at another native packet')
                payload=bytes.fromhex(source['body_hex']); vehicle, offset = PacketTraceProxy.varint(payload)
                count,width=PacketTraceProxy.varint(payload[offset:]);offset+=width
                ids=[]
                for _ in range(count):
                    identity,width=PacketTraceProxy.varint(payload[offset:]);offset+=width;ids.append(identity)
                if offset!=len(payload) or vehicle!=cart['id']['native_id'] or ids!=passengers['value']:
                    raise RuntimeError('vehicle fields differ from original native passenger list')
                if (observation['player_native_id'] in ids)!=wanted_mounted:
                    raise RuntimeError('native passenger membership differs from claimed relationship')
                source_proof.append({k:source[k] for k in ('connection','ordinal','packet_id','body_sha256')})
                return source
            mount_source = verify(mounted['vehicle'], True)
            unmount_source = verify(completed['vehicle'], False)
            mount = mounted['vehicle']['relation']['value']['mount']
            if mount != requested['request']['id']['mount'] or observed['id'] != completed['complete']['id']:
                raise RuntimeError('owned dismount changed its original identity')
            if completed['vehicle']['relation']['value']['previous_mount'] != mount or observed['observed_unmounted'] != completed['vehicle']['relation']:
                raise RuntimeError('actual absence is not bound to original continuous mount')
            if not (mount_source['ordinal'] < inputs[-2]['ordinal'] < unmount_source['ordinal'] < inputs[-1]['ordinal']):
                raise RuntimeError('native neutral was not sent after the original actual unmount receipt')
            result['source_proof']=source_proof
            result['operation_frames']=frames
            result['authority_limits']='Same common Client/session per mode: one original empty-hand INTERACT mounts the received minecart UUID; original player RootVehicle.Attach independently confirms native riding. Owned request is sent once, actual same-vehicle SET_PASSENGERS absence precedes one explicit neutral, native RootVehicle is independently absent, duplicate requests/releases emit no extra input, and ground preview remains refused. Original input bytes and received passenger fields/ordinals are checked. No causal server ACK, vehicle physics/control or ground continuation is inferred.'
            if controlling:
                result['authority_limits']='Same common Client/mode/session: finite dry ground approach, original INTERACT and actual continuous mount, one owned 14-frame digital control run ending in explicit neutral, independently observed original minecart displacement, original approach retired without restoring ground authority, then owned dismount/actual absence/explicit neutral, stale mount control refused, disconnect retaining both histories. Exactly one interaction and 16 original input frames. Input submission and fresh received vehicle position are separate facts, with no vehicle stop or control ACK. Paddles, special interpolation and general vehicle physics remain B6.'
            if controlling:
                ground_boundary=trace.mark()
                ground=stage(probe,messages,'b6_vehicle_ground',result['records'])['value']
                stop=ground['record']['grounding']['motion']
                if stop['status']!='predicted' or stop['dispatched_ticks']!=2 or stop['attempted_tick']!=2:
                    raise RuntimeError('dismount ground stop not fully dispatched')
                if ground['record']['id']!=completed['complete']['id'] or ground['before']['session']!=baseline['player']['session']:
                    raise RuntimeError('ground continuation changed original Client or dismount')
                if ground['before']['received_pose']!=ground['after']['received_pose']:
                    raise RuntimeError('ground stop rewrote or lost original received pose')
                pose=ground['before']['received_pose'];actual=stop['preview']['initial']['position']
                if actual['source']!={'kind':'received','sequence':pose['receive_sequence']} or actual['value']!=pose['position']:
                    raise RuntimeError('ground start is not the actual received dismount position')
                peers=[f for f in trace.since(0) if f['connection']==connection and f['direction']=='clientbound' and f['phase'] in ('configuration','play')]
                source=peers[pose['receive_sequence']-1]
                if source['packet_id']!=(0x35 if version=='1.16.1' else 0x46):raise RuntimeError('ground pose source is not original own position')
                raw=bytes.fromhex(source['body_hex']);offset=0 if version=='1.16.1' else PacketTraceProxy.varint(raw)[1]
                if list(struct.unpack_from('>ddd',raw,offset))!=pose['position']:raise RuntimeError('ground pose differs from original absolute packet')
                result['ground_pose_source']={k:source[k] for k in ('connection','ordinal','packet_id','body_sha256')}
                position_id=0x13 if version=='1.16.1' else 0x1e
                def sent_positions(boundary,count):
                    frames=trace.since(boundary)
                    return frames if len([f for f in frames if f['direction']=='serverbound' and f['phase']=='play' and f['packet_id']==position_id])>=count else None
                ground_frames=until(lambda:sent_positions(ground_boundary,2))
                positions=[f for f in ground_frames if f['direction']=='serverbound' and f['phase']=='play' and f['packet_id']==position_id]
                if len(positions)!=2:raise RuntimeError('ground stop duplicated native position frames')
                for frame,expected in zip(positions,stop['preview']['frames']):
                    if list(struct.unpack_from('>ddd',bytes.fromhex(frame['body_hex'])))!=expected['position']:
                        raise RuntimeError('ground stop position differs from declared model')
                if version!='1.16.1' and [f['body_hex'] for f in ground_frames if f['direction']=='serverbound' and f['phase']=='play' and f['packet_id']==input_id]!=['00','00']:
                    raise RuntimeError('ground stop did not send exactly two released inputs')
                result['ground_frames']=ground_frames
                walk_boundary=trace.mark()
                walk=stage(probe,messages,'b6_vehicle_ground_walk',result['records'])['value']
                expected=walk['motion']['preview']['frames'][-1]['position']
                if walk['motion']['dispatched_ticks']!=15 or walk['motion']['run_id']<=stop['run_id']:
                    raise RuntimeError('new finite ground run did not follow the stop')
                if expected[2]<=pose['position'][2]+0.05:raise RuntimeError('ground continuation did not move forward')
                def native_ground_position():
                    raw=rcon.command('data get entity UnifiedProbe Pos')
                    return raw if max(abs(a-b) for a,b in zip(pos(raw),expected))<1e-5 else None
                result['native_ground_position']=until(native_ground_position)
                result['ground_walk_frames']=until(lambda:sent_positions(walk_boundary,15))
                storage_boundary=trace.mark()
                storage=stage(probe,messages,'b6_vehicle_ground_storage',result['records'])['value']
                opening=storage['opening']['observed_screen']['id']
                peers=[f for f in trace.since(0) if f['connection']==connection and f['direction']=='clientbound' and f['phase'] in ('configuration','play')]
                actual_open=peers[opening['opened_sequence']-1]
                if actual_open['packet_id']!=(0x2e if version=='1.16.1' else 0x39):raise RuntimeError('ground storage source is not original OPEN')
                raw=bytes.fromhex(actual_open['body_hex']);window,offset=PacketTraceProxy.varint(raw);menu,_=PacketTraceProxy.varint(raw[offset:])
                if window!=opening['window'] or menu!=2:raise RuntimeError('ground storage is not the expected native 27-slot chest')
                result['ground_storage_open_source']={k:actual_open[k] for k in ('connection','ordinal','packet_id','body_sha256')}
                close_id=0x0a if version=='1.16.1' else 0x12
                def ground_storage_closed():
                    frames=trace.since(storage_boundary)
                    return frames if any(f['direction']=='serverbound' and f['phase']=='play' and f['packet_id']==close_id for f in frames) else None
                result['ground_storage_frames']=until(ground_storage_closed)
                result['native_ground_chest']=rcon.command('data get block 3 65 3 Items')
                if outer_snbt_compounds(result['native_ground_chest']):raise RuntimeError('ground storage altered chest')
                result['authority_limits']+=' Explicit resume_ground binds actual completed dismount and fresh received pose to declared local zero seed, dispatches two released model ticks, then a new 15-tick forward ground run and actual chest OPEN/close/hotbar selection on the same Client. Original pose and independent native endpoint are checked. This is not a received zero velocity, server rest ACK or general vehicle physics.'
            trace.expect_disconnect()
            stage(probe, messages, 'a5_vehicle_disconnect', result['records'])
            probe.wait(timeout=10); reader.join(timeout=2)
            if probe.returncode!=0 or reader.is_alive(): raise RuntimeError('vehicle probe failed at disconnect')
            result['result']='passed'
        finally:
            if probe.poll() is None:
                probe.terminate()
                try: probe.wait(timeout=10)
                except subprocess.TimeoutExpired: probe.kill(); probe.wait(timeout=5)


def run_furnace(version, env, rcon, trace, report, probe_log, stderr_log):
    """A5 common slots: native fuel-first input, real smelting, output, close.
    After baseline all RCON commands are read-only, and JVMs stay sequential.
    """
    results=report['native_results']['furnace']={}
    def items(command, expected):
        raw=rcon.command(command);stacks=outer_snbt_compounds(raw)
        if len(stacks)!=len(expected):return None
        for slot,(item,count) in expected.items():
            if not any(re.search(rf'Slot: {slot}b(?:,|\s|}})',stack)
                and f'id: "minecraft:{item}"' in stack
                and re.search(rf'(?:Count|count): {count}(?:b)?(?:,|\s|}})',stack) for stack in stacks):return None
        return raw
    for mode in ('survival','creative'):
        until(lambda: matched(rcon.command('execute unless entity @a[name=UnifiedProbe]'),'Test passed'))
        result=results[mode]={'fixture':{},'records':[]}
        for command in ['setblock 0 65 1 minecraft:air','setblock 0 65 2 minecraft:air','setblock 0 65 2 minecraft:furnace[facing=north,lit=false]']:
            result['fixture'][command]=rcon.command(command)
        probe=subprocess.Popen([str(REPO/'target/debug/examples/common_native_probe')],cwd=REPO,
            env=dict(env,VOXRIG_NATIVE_SCENARIO='furnace',VOXRIG_NATIVE_MODE=mode),stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=stderr_log,text=True,bufsize=1)
        messages=queue.Queue();reader=threading.Thread(target=pump,args=(probe.stdout,messages,probe_log),daemon=True);reader.start()
        try:
            stage(probe,messages,'a5_furnace_ready',result['records'])
            commands=['gamemode '+mode+' UnifiedProbe','tp UnifiedProbe 0.5 65 0.5 0 30','clear UnifiedProbe']
            for slot,item in [('inventory.0','coal'),('inventory.1','iron_ore')]:
                commands.append(f'replaceitem entity UnifiedProbe {slot} minecraft:{item} 1' if version=='1.16.1' else f'item replace entity UnifiedProbe {slot} with minecraft:{item} 1')
            for command in commands:result['fixture'][command]=rcon.command(command)
            baseline=stage(probe,messages,'a5_furnace_baseline',result['records'])['value']
            result['native_before']=until(lambda:items('data get entity UnifiedProbe Inventory',{9:('coal',1),10:('iron_ore',1)}))
            result['position_before']=rcon.command('data get entity UnifiedProbe Pos')
            boundary=trace.mark()
            opened=stage(probe,messages,'a5_furnace_open',result['records'])['value']
            loaded=stage(probe,messages,'a5_furnace_load',result['records'])['value']
            result['native_input']=until(lambda:items('data get block 0 65 2 Items',{0:('iron_ore',1)}))
            result['native_running']=until(lambda:matched(rcon.command('execute if block 0 65 2 minecraft:furnace[lit=true]'),'Test passed'))
            output=stage(probe,messages,'a5_furnace_output',result['records'],timeout=40)['value']
            result['native_output']=until(lambda:items('data get block 0 65 2 Items',{2:('iron_ingot',1)}))
            taken=stage(probe,messages,'a5_furnace_take',result['records'])['value']
            result['native_after']=until(lambda:items('data get entity UnifiedProbe Inventory',{11:('iron_ingot',1)}))
            result['native_empty']=until(lambda:items('data get block 0 65 2 Items',{}))
            result['position_after']=rcon.command('data get entity UnifiedProbe Pos')
            if result['position_after']!=result['position_before']:raise RuntimeError('furnace slot workflow moved player')
            closed=stage(probe,messages,'a5_furnace_close',result['records'])['value']
            furnace_id=opened['furnace']['screen']['id'];session=baseline['session']
            if any(v['session']!=session or v['screen']['id']!=furnace_id for v in [opened['furnace'],loaded['furnace'],output,taken['furnace']]):raise RuntimeError('furnace workflow changed session/opening')
            frames=trace.since(boundary);click_id=0x09 if version=='1.16.1' else 0x11
            clicks=[f for f in frames if f['direction']=='serverbound' and f['phase']=='play' and f['packet_id']==click_id]
            if len(clicks)!=6:raise RuntimeError('expected exactly six furnace PICKUP frames, got '+str(len(clicks)))
            for f in clicks:
                body=bytes.fromhex(f['body_hex']);window,width=PacketTraceProxy.varint(body) if version!='1.16.1' else (body[0],1)
                if window!=furnace_id['window']:raise RuntimeError('furnace click used another window')
            result['operation_frames']=frames
            result['authority_limits']='Same public consumer/Client per mode: constructor-derived slots, four fresh load click receipts, native burning and iron_ore->iron_ingot, two fresh output/store receipts, exactly six PICKUP frames, independent RCON inventory/furnace contents and unchanged position. Close dispatch does not forge a screen echo; old opening cannot emit another click. Fuel is loaded before input so it remains receivable. Concurrent smelting changes may require inspection; no automatic retry, cook-time/XP forecast or recipe planning.'
            trace.expect_disconnect();stage(probe,messages,'a5_furnace_disconnect',result['records'])
            probe.wait(timeout=10);reader.join(timeout=2)
            if probe.returncode!=0 or reader.is_alive():raise RuntimeError('furnace probe failed at disconnect')
            result['result']='passed'
        finally:
            if probe.poll() is None:
                probe.terminate()
                try:probe.wait(timeout=10)
                except subprocess.TimeoutExpired:probe.kill();probe.wait(timeout=5)


def run_basic_workflow(version, env, rcon, trace, report, probe_log, stderr_log):
    """One connection per mode; after baseline the driver only reads native state."""
    results = report['native_results']['basic_workflow'] = {}
    def exact_items(command, expected):
        response = rcon.command(command)
        stacks = outer_snbt_compounds(response)
        if len(stacks) != len(expected):
            return None
        for slot, (item, count) in expected.items():
            if not any(re.search(rf'Slot: {slot}b(?:,|\s|}})', stack)
                       and f'id: "minecraft:{item}"' in stack
                       and re.search(rf'(?:Count|count): {count}(?:b)?(?:,|\s|}})', stack)
                       for stack in stacks):
                return None
        return response
    def inventory(expected):
        return exact_items('data get entity UnifiedProbe Inventory', expected)
    for mode in ('survival', 'creative'):
        until(lambda: matched(rcon.command('execute unless entity @a[name=UnifiedProbe]'), 'Test passed'))
        result = results[mode] = {'fixture':{}, 'records':[]}
        for command in ['setblock 0 65 1 minecraft:air', 'setblock 0 65 2 minecraft:chest[facing=north,type=single,waterlogged=false]',
                        'data merge block 0 65 2 {Items:[]}', 'setblock 2 65 0 minecraft:air']:
            result['fixture'][command] = rcon.command(command)
        probe = subprocess.Popen([str(REPO / 'target/debug/examples/common_native_probe')], cwd=REPO,
            env=dict(env, VOXRIG_NATIVE_SCENARIO='basic-workflow'), stdin=subprocess.PIPE,
            stdout=subprocess.PIPE, stderr=stderr_log, text=True, bufsize=1)
        messages = queue.Queue()
        reader = threading.Thread(target=pump, args=(probe.stdout, messages, probe_log), daemon=True)
        reader.start()
        try:
            stage(probe, messages, 'workflow_ready', result['records'])
            commands = ['gamemode '+mode+' UnifiedProbe', 'tp UnifiedProbe 0.5 65 0.5 0 0', 'clear UnifiedProbe']
            for slot, item, count in [('inventory.0','oak_planks',2), ('inventory.2','stone',2), ('hotbar.1','dirt',3)]:
                commands.append(f'replaceitem entity UnifiedProbe {slot} minecraft:{item} {count}' if version == '1.16.1'
                    else f'item replace entity UnifiedProbe {slot} with minecraft:{item} {count}')
            for command in commands:
                result['fixture'][command] = rcon.command(command)
            baseline = stage(probe, messages, 'workflow_baseline', result['records'])['value']
            result['baseline_inventory'] = until(lambda:inventory({9:('oak_planks',2),11:('stone',2),1:('dirt',3)}))
            result['baseline_position'] = rcon.command('data get entity UnifiedProbe Pos')
            boundary = trace.mark()
            moved = stage(probe, messages, 'workflow_move', result['records'])['value']
            expected = moved['position']['value']
            def endpoint():
                raw = rcon.command('data get entity UnifiedProbe Pos')
                actual = [float(v) for v in re.findall(r'(-?\d+(?:\.\d+)?(?:[Ee][+-]?\d+)?)d', raw)]
                return raw if len(actual)==3 and all(abs(a-b)<1e-7 for a,b in zip(actual,expected)) else None
            result['moved_position'] = until(endpoint)
            if abs(expected[0]-0.5) < 0.1 or abs(expected[1]-65.0)>1e-7:
                raise RuntimeError('workflow did not make a grounded horizontal movement')
            stored = stage(probe, messages, 'workflow_storage', result['records'])['value']
            result['storage_items'] = until(lambda:exact_items('data get block 0 65 2 Items',{0:('stone',2)}))
            result['storage_inventory'] = until(lambda:inventory({9:('oak_planks',2),1:('dirt',3)}))
            crafted = stage(probe, messages, 'workflow_craft', result['records'])['value']
            result['crafted_inventory'] = until(lambda:inventory({10:('stick',4),1:('dirt',3)}))
            placed = stage(probe, messages, 'workflow_place', result['records'])['value']
            result['placed_block'] = until(lambda:matched(rcon.command('execute if block 2 65 0 minecraft:dirt'),'Test passed'))
            result['final_inventory'] = until(lambda:inventory({10:('stick',4),1:('dirt',2 if mode=='survival' else 3)}))
            result['final_position'] = until(endpoint)
            stamp = baseline['player']['session']
            if any(p['session'] != stamp for p in (moved,stored['player'],crafted['player'],placed['player'])):
                raise RuntimeError('workflow changed connection/world during operations')
            for player in (stored['player'],crafted['player'],placed['player']):
                if player['inventory']['cursor']['value']['kind'] != 'empty':
                    raise RuntimeError('workflow left a carried stack')
            # Retained actual foreign OPEN is honest history after no-echo close.
            # SubmittedClose is the separate local basis for player-menu operations.
            if not stored['close']['dispatched'] or stored['close']['id']['screen'] != stored['open']['observed_screen']['id']:
                raise RuntimeError('workflow close was not bound to the actual opening')
            result['operation_frames'] = trace.since(boundary)
            result['authority_limits'] = 'One unchanged Client per mode; dry ordinary-items fixture only. Native position, chest contents, ingredient/output totals and placed block verified independently through RCON. Client retains actual screen/cursor/slot/block receipts and separately records predicted movement/local close. No mining continuation, recipe-book placement or full feature parity is claimed.'
            trace.expect_disconnect()
            stage(probe, messages, 'workflow_disconnect', result['records'])
            probe.wait(timeout=10)
            if probe.returncode != 0:
                raise RuntimeError('workflow probe failed after disconnect')
            reader.join(timeout=2)
            if reader.is_alive():
                raise RuntimeError('workflow probe reader did not finish')
            result['result'] = 'passed'
        finally:
            if probe.poll() is None:
                probe.terminate()
                try:
                    probe.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    probe.kill(); probe.wait(timeout=5)



def run_equipment_entity(version, env, rcon, trace, report, probe_log, stderr_log):
    """A2: received equipment transfer, one attack and one villager interaction."""
    results = report['native_results']['equipment_entity'] = {}
    selector = '@e[type=minecraft:sheep,tag=VoxrigA2Target,limit=1]'
    for mode in ('survival','creative'):
        until(lambda:matched(rcon.command('execute unless entity @a[name=UnifiedProbe]'),'Test passed'))
        result = results[mode] = {'fixture':{},'records':[]}
        for command in ['kill @e[tag=VoxrigA2Target]', 'kill @e[tag=VoxrigA2Merchant]',
                        'setblock 0 65 1 minecraft:air',
                        'summon minecraft:sheep 2.5 65 0.5 {NoAI:1b,Tags:["VoxrigA2Target"]}',
                        'summon minecraft:villager 0.5 65 2.5 {NoAI:1b,Tags:["VoxrigA2Merchant"],VillagerData:{type:"minecraft:plains",profession:"minecraft:farmer",level:1}}']:
            result['fixture'][command] = rcon.command(command)
        probe = subprocess.Popen([str(REPO/'target/debug/examples/common_native_probe')],cwd=REPO,
            env=dict(env,VOXRIG_NATIVE_SCENARIO='equipment-entity'),stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=stderr_log,text=True,bufsize=1)
        messages = queue.Queue()
        reader = threading.Thread(target=pump,args=(probe.stdout,messages,probe_log),daemon=True)
        reader.start()
        try:
            stage(probe,messages,'a2_ready',result['records'])
            commands = ['gamemode '+mode+' UnifiedProbe','tp UnifiedProbe 0.5 65 0.5 0 0','clear UnifiedProbe']
            commands.append('replaceitem entity UnifiedProbe hotbar.2 minecraft:iron_boots 1' if version=='1.16.1'
                else 'item replace entity UnifiedProbe hotbar.2 with minecraft:iron_boots 1')
            for command in commands:
                result['fixture'][command] = rcon.command(command)
            baseline = stage(probe,messages,'a2_baseline',result['records'])['value']
            result['inventory_before'] = rcon.command('data get entity UnifiedProbe Inventory')
            mark = trace.mark()
            equipped = stage(probe,messages,'a2_equip',result['records'])['value']
            def boots():
                raw = rcon.command('data get entity UnifiedProbe Inventory' if version=='1.16.1' else 'data get entity UnifiedProbe equipment.feet')
                if version=='1.16.1':
                    stacks = outer_snbt_compounds(raw)
                    found = [stack for stack in stacks if re.search(r'Slot: 100b(?:,|\s|})',stack) and 'id: "minecraft:iron_boots"' in stack]
                    return raw if len(stacks)==1 and len(found)==1 else None
                return raw if 'id: "minecraft:iron_boots"' in raw and re.search(r'count: 1(?:,|\s|})',raw) else None
            result['equipment_feet'] = until(boots)
            def health():
                raw = rcon.command('data get entity '+selector+' Health')
                match = re.search(r': (-?\d+(?:\.\d+)?)f$',raw)
                if not match: raise RuntimeError('missing native sheep health: '+raw)
                return float(match.group(1)),raw
            before,before_raw = health()
            result['health_before'] = before_raw
            attacked = stage(probe,messages,'a2_attack',result['records'])['value']
            def damaged():
                value,raw = health()
                return raw if 0<value<before else None
            result['health_after'] = until(damaged)
            result['retire_fixture'] = rcon.command('kill @e[tag=VoxrigA2Target]')
            retire_mark = trace.mark()
            retired = stage(probe,messages,'a2_retire',result['records'])['value']
            entity_packet = 0x0e if version=='1.16.1' else 0x19
            if any(f['direction']=='serverbound' and f['phase']=='play' and f['packet_id']==entity_packet for f in trace.since(retire_mark)):
                raise RuntimeError('retired entity request emitted a frame')
            interacted = stage(probe,messages,'a2_interact',result['records'])['value']
            stamp = baseline['player']['session']
            if any(s!=stamp for s in (baseline['spawns']['session'],equipped['player']['session'],attacked['target']['session'],retired['spawns']['session'],interacted['target']['session'],interacted['screen']['session'])):
                raise RuntimeError('A2 changed connection/world across equipment/entity operations')
            result['operation_frames'] = trace.since(mark)
            frames = [f for f in result['operation_frames'] if f['direction']=='serverbound' and f['phase']=='play' and f['packet_id']==entity_packet]
            if len(frames)!=2: raise RuntimeError('A2 expected one attack and one interaction frame')
            for frame,target,action in [(frames[0],attacked['target'],1),(frames[1],interacted['target'],0)]:
                raw = bytes.fromhex(frame['body_hex']); native_id,offset = PacketTraceProxy.varint(raw)
                if native_id!=target['native_id'] or raw[offset:]!=(bytes([1,0]) if action==1 else bytes([0,0,0])):
                    raise RuntimeError('native entity request fields differ from the public intent')
            result['authority_limits'] = 'One Client/session per mode. Actual equipment slot receipt plus independent native feet storage; one empty-hand attack changes native sheep health; one empty-hand interaction precedes actual merchant OPEN. Dispatch is not damage/trade confirmation. Spawn positions are historical, no current movement/metadata/hitbox/reach or tactics are inferred. Merchant layout/trading remain unsupported. Removed targets emit no entity frame.'
            trace.expect_disconnect()
            stage(probe,messages,'a2_disconnect',result['records'])
            probe.wait(timeout=10); reader.join(timeout=2)
            if probe.returncode!=0 or reader.is_alive(): raise RuntimeError('A2 probe failed at disconnect')
            result['result'] = 'passed'
        finally:
            if probe.poll() is None:
                probe.terminate()
                try: probe.wait(timeout=10)
                except subprocess.TimeoutExpired: probe.kill(); probe.wait(timeout=5)


def run_recipe_placement(version, env, rcon, trace, report, probe_log, stderr_log):
    results=report['native_results']['recipe_placement']={}
    for mode in ('survival','creative'):
      for ui in ('player','table'):
       for amount in ('next','maximum'):
        until(lambda:matched(rcon.command('execute unless entity @a[name=UnifiedProbe]'),'Test passed'))
        result=results[mode+'-'+ui+'-'+amount]={'fixture':{},'records':[]}
        for command in ['setblock 0 65 1 minecraft:air', 'setblock 0 65 2 minecraft:crafting_table' if ui=='table' else 'setblock 0 65 2 minecraft:air']:
            result['fixture'][command]=rcon.command(command)
        probe=subprocess.Popen([str(REPO/'target/debug/examples/common_native_probe')],cwd=REPO,
            env=dict(env,VOXRIG_NATIVE_SCENARIO='recipe-placement',VOXRIG_NATIVE_RECIPE_MODE=mode,VOXRIG_NATIVE_RECIPE_UI=ui,VOXRIG_NATIVE_RECIPE_AMOUNT=amount),stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=stderr_log,text=True,bufsize=1)
        messages=queue.Queue();reader=threading.Thread(target=pump,args=(probe.stdout,messages,probe_log),daemon=True);reader.start()
        try:
            stage(probe,messages,'b4_recipe_ready',result['records'])
            commands=['gamemode '+mode+' UnifiedProbe','tp UnifiedProbe 0.5 65 0.5 0 0','clear UnifiedProbe']
            commands.append('replaceitem entity UnifiedProbe inventory.0 minecraft:oak_planks 10' if version=='1.16.1' else 'item replace entity UnifiedProbe inventory.0 with minecraft:oak_planks 10')
            commands.append('recipe give UnifiedProbe *')
            for command in commands:result['fixture'][command]=rcon.command(command)
            boundary=trace.mark()
            done=stage(probe,messages,'b4_recipe_run',result['records'],timeout=120)['value']
            expected={10:('stick',20 if amount=='maximum' else 4)}
            if amount=='next':expected[9]=('oak_planks',8)
            def inventory():
                response=rcon.command('data get entity UnifiedProbe Inventory');stacks=outer_snbt_compounds(response)
                if len(stacks)!=len(expected):return None
                for slot,(item,count) in expected.items():
                    if not any(re.search(rf'Slot: {slot}b(?:,|\s|}})',s) and f'id: "minecraft:{item}"' in s and re.search(rf'(?:Count|count): {count}(?:b)?(?:,|\s|}})',s) for s in stacks):return None
                return response
            result['native_inventory']=until(inventory)
            result['native_position']=rcon.command('data get entity UnifiedProbe Pos')
            if '[0.5d, 65.0d, 0.5d]' not in result['native_position']:raise RuntimeError('recipe placement moved native actor')
            result['operation_frames']=trace.since(boundary)
            requests=[f for f in result['operation_frames'] if f['phase']=='play' and f['direction']=='serverbound' and f['packet_id']==(0x19 if version=='1.16.1' else 0x26)]
            if len(requests)!=1:raise RuntimeError('recipe placement request was repeated or missing')
            raw=bytes.fromhex(requests[0]['body_hex'])
            if raw[-1]!=int(amount=='maximum'):raise RuntimeError('native recipe amount flag differs')
            if not done['placement']['send']['dispatched'] or done['placement']['stage']!='observed_placed':raise RuntimeError('missing actual recipe completion')
            trace.expect_disconnect();stage(probe,messages,'b4_recipe_disconnect',result['records'])
            if probe.wait(timeout=15)!=0:raise RuntimeError('recipe placement consumer failed')
            result['authority_limits']='One unchanged common Client per mode/UI/amount: native recipe request -> actual conserved input/inventory capture -> explicit single result takes and actual cursor deposits -> empty grid/cursor -> original table close -> disconnect. Fixture only before operation; RCON thereafter reads exact native ingredient/output counts and unchanged position. No native type-tie attribution, automatic shift-crafting, ghost or nonempty-result-cursor support claimed.'
            result['result']='passed'
        finally:
            if probe.poll() is None:
                probe.terminate()
                try:probe.wait(timeout=10)
                except subprocess.TimeoutExpired:probe.kill();probe.wait(timeout=5)



def run_recipe_result_merge(version, env, rcon, trace, report, probe_log, stderr_log):
    results=report['native_results']['recipe_result_merge']={}
    for mode in ('survival','creative'):
      for ui in ('player','table'):
       for amount in ('maximum',):
        until(lambda:matched(rcon.command('execute unless entity @a[name=UnifiedProbe]'),'Test passed'))
        result=results[mode+'-'+ui+'-'+amount]={'fixture':{},'records':[]}
        for command in ['setblock 0 65 1 minecraft:air', 'setblock 0 65 2 minecraft:crafting_table' if ui=='table' else 'setblock 0 65 2 minecraft:air']:
            result['fixture'][command]=rcon.command(command)
        probe=subprocess.Popen([str(REPO/'target/debug/examples/common_native_probe')],cwd=REPO,
            env=dict(env,VOXRIG_NATIVE_SCENARIO='recipe-result-merge',VOXRIG_NATIVE_RECIPE_MODE=mode,VOXRIG_NATIVE_RECIPE_UI=ui,VOXRIG_NATIVE_RECIPE_AMOUNT=amount),stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=stderr_log,text=True,bufsize=1)
        messages=queue.Queue();reader=threading.Thread(target=pump,args=(probe.stdout,messages,probe_log),daemon=True);reader.start()
        try:
            stage(probe,messages,'b4_recipe_ready',result['records'])
            commands=['gamemode '+mode+' UnifiedProbe','tp UnifiedProbe 0.5 65 0.5 0 0','clear UnifiedProbe']
            commands.append('replaceitem entity UnifiedProbe inventory.0 minecraft:oak_planks 4' if version=='1.16.1' else 'item replace entity UnifiedProbe inventory.0 with minecraft:oak_planks 4')
            commands.append('replaceitem entity UnifiedProbe inventory.2 minecraft:stick 60' if version=='1.16.1' else 'item replace entity UnifiedProbe inventory.2 with minecraft:stick 60')
            commands.append('recipe give UnifiedProbe *')
            for command in commands:result['fixture'][command]=rcon.command(command)
            boundary=trace.mark()
            done=stage(probe,messages,'b4_recipe_run',result['records'],timeout=120)['value']
            expected={10:('stick',64),11:('stick',4)}
            def inventory():
                response=rcon.command('data get entity UnifiedProbe Inventory');stacks=outer_snbt_compounds(response)
                if len(stacks)!=len(expected):return None
                for slot,(item,count) in expected.items():
                    if not any(re.search(rf'Slot: {slot}b(?:,|\s|}})',s) and f'id: "minecraft:{item}"' in s and re.search(rf'(?:Count|count): {count}(?:b)?(?:,|\s|}})',s) for s in stacks):return None
                return response
            result['native_inventory']=until(inventory)
            result['native_position']=rcon.command('data get entity UnifiedProbe Pos')
            if '[0.5d, 65.0d, 0.5d]' not in result['native_position']:raise RuntimeError('recipe placement moved native actor')
            result['operation_frames']=trace.since(boundary)
            requests=[f for f in result['operation_frames'] if f['phase']=='play' and f['direction']=='serverbound' and f['packet_id']==(0x19 if version=='1.16.1' else 0x26)]
            if len(requests)!=1:raise RuntimeError('recipe placement request was repeated or missing')
            raw=bytes.fromhex(requests[0]['body_hex'])
            if raw[-1]!=int(amount=='maximum'):raise RuntimeError('native recipe amount flag differs')
            if not done['placement']['send']['dispatched'] or done['placement']['stage']!='observed_placed':raise RuntimeError('missing actual recipe completion')
            takes=done['takes']
            if len(takes)!=2 or any(t['stage']!='observed_taken' or t['requires_inspection'] is not None for t in takes):
                raise RuntimeError('expected two actual entire-result takes')
            first=takes[0]
            if [first[k]['value']['item']['count'] for k in ('cursor_before','cursor_prediction','cursor_receipt')]!=[60,64,64] or not done['overflow_rejected']:
                raise RuntimeError('actual held merge/capacity refusal evidence differs')
            clicks=[f for f in result['operation_frames'] if f['phase']=='play' and f['direction']=='serverbound' and f['packet_id']==(0x09 if version=='1.16.1' else 0x11)]
            if len(clicks)!=5:raise RuntimeError('expected pickup, two result takes and two deposits; overflow must emit no click')
            trace.expect_disconnect();stage(probe,messages,'b4_recipe_disconnect',result['records'])
            if probe.wait(timeout=15)!=0:raise RuntimeError('recipe placement consumer failed')
            result['authority_limits']='One common Client per mode/UI: native Maximum recipe placement, actual stick60 pickup, entire result4 merged to actual cursor64, second result rejected before I/O because it cannot wholly fit, explicit deposit, fresh empty-cursor take/deposit, empty grid/cursor, original table close, selection, disconnect. Exact independent native inventory64+4 and unchanged position; five click frames prove no overflow request or replay. Native full grid/result receipts confirm consumption and regeneration; no partial or shift crafting inferred.'
            result['result']='passed'
        finally:
            if probe.poll() is None:
                probe.terminate()
                try:probe.wait(timeout=10)
                except subprocess.TimeoutExpired:probe.kill();probe.wait(timeout=5)


def run_recipe_result_transfer(version, env, rcon, trace, report, probe_log, stderr_log):
    results=report['native_results']['recipe_result_transfer']={}
    for mode in ('survival','creative'):
      for ui in ('player','table'):
       for amount in ('next','maximum'):
        until(lambda:matched(rcon.command('execute unless entity @a[name=UnifiedProbe]'),'Test passed'))
        result=results[mode+'-'+ui+'-'+amount]={'fixture':{},'records':[]}
        for command in ['setblock 0 65 1 minecraft:air', 'setblock 0 65 2 minecraft:crafting_table' if ui=='table' else 'setblock 0 65 2 minecraft:air']:
            result['fixture'][command]=rcon.command(command)
        probe=subprocess.Popen([str(REPO/'target/debug/examples/common_native_probe')],cwd=REPO,
            env=dict(env,VOXRIG_NATIVE_SCENARIO='recipe-result-transfer',VOXRIG_NATIVE_RECIPE_MODE=mode,VOXRIG_NATIVE_RECIPE_UI=ui,VOXRIG_NATIVE_RECIPE_AMOUNT=amount),stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=stderr_log,text=True,bufsize=1)
        messages=queue.Queue();reader=threading.Thread(target=pump,args=(probe.stdout,messages,probe_log),daemon=True);reader.start()
        try:
            stage(probe,messages,'b4_recipe_ready',result['records'])
            commands=['gamemode '+mode+' UnifiedProbe','tp UnifiedProbe 0.5 65 0.5 0 0','clear UnifiedProbe']
            commands.append('replaceitem entity UnifiedProbe inventory.0 minecraft:oak_planks 6' if version=='1.16.1' else 'item replace entity UnifiedProbe inventory.0 with minecraft:oak_planks 6')
            commands.append('replaceitem entity UnifiedProbe inventory.2 minecraft:stick 60' if version=='1.16.1' else 'item replace entity UnifiedProbe inventory.2 with minecraft:stick 60')
            commands.append('replaceitem entity UnifiedProbe inventory.3 minecraft:dirt 1' if version=='1.16.1' else 'item replace entity UnifiedProbe inventory.3 with minecraft:dirt 1')
            commands.append('recipe give UnifiedProbe *')
            for command in commands:result['fixture'][command]=rcon.command(command)
            boundary=trace.mark()
            done=stage(probe,messages,'b4_recipe_run',result['records'],timeout=120)['value']
            expected=({8:('stick',8),11:('stick',64),12:('dirt',1)} if amount=='maximum' else {9:('oak_planks',4),11:('stick',64),12:('dirt',1)})
            def inventory():
                response=rcon.command('data get entity UnifiedProbe Inventory');stacks=outer_snbt_compounds(response)
                if len(stacks)!=len(expected):return None
                for slot,(item,count) in expected.items():
                    if not any(re.search(rf'Slot: {slot}b(?:,|\s|}})',s) and f'id: "minecraft:{item}"' in s and re.search(rf'(?:Count|count): {count}(?:b)?(?:,|\s|}})',s) for s in stacks):return None
                return response
            result['native_inventory']=until(inventory)
            result['native_position']=rcon.command('data get entity UnifiedProbe Pos')
            if '[0.5d, 65.0d, 0.5d]' not in result['native_position']:raise RuntimeError('recipe placement moved native actor')
            result['operation_frames']=trace.since(boundary)
            requests=[f for f in result['operation_frames'] if f['phase']=='play' and f['direction']=='serverbound' and f['packet_id']==(0x19 if version=='1.16.1' else 0x26)]
            if len(requests)!=1:raise RuntimeError('recipe placement request was repeated or missing')
            raw=bytes.fromhex(requests[0]['body_hex'])
            if raw[-1]!=int(amount=='maximum'):raise RuntimeError('native recipe amount flag differs')
            if not done['placement']['send']['dispatched'] or done['placement']['stage']!='observed_placed':raise RuntimeError('missing actual recipe completion')
            transfer=done['transfer']
            if transfer['stage']!='observed_transferred' or transfer['requires_inspection'] is not None or transfer['destination']!='inventory':
                raise RuntimeError('actual native result transfer missing')
            if transfer['inventory_output_increase']!=(12 if amount=='maximum' else 4) or transfer['inventory_after'] is None or transfer['cursor_receipt']['value']!=transfer['cursor_before']['value']:
                raise RuntimeError('actual inventory gain or unchanged cursor differs')
            clicks=[f for f in result['operation_frames'] if f['phase']=='play' and f['direction']=='serverbound' and f['packet_id']==(0x09 if version=='1.16.1' else 0x11)]
            if len(clicks)!=(1 if amount=='maximum' else 3):raise RuntimeError('native shift or ordinary cursor moves were repeated or missing')
            quick=[]
            for click in clicks:
                body=bytes.fromhex(click['body_hex'])
                if version=='1.16.1':
                    slot=int.from_bytes(body[1:3],'big',signed=True);mode_id=body[6]
                else:
                    _,off=trace.varint(body);_,size=trace.varint(body[off:]);off+=size
                    slot=int.from_bytes(body[off:off+2],'big',signed=True);mode_id,_=trace.varint(body[off+3:])
                if mode_id==1:quick.append(slot)
            if quick!=[0]:raise RuntimeError('expected exactly one original result QUICK_MOVE')
            trace.expect_disconnect();stage(probe,messages,'b4_recipe_disconnect',result['records'])
            if probe.wait(timeout=15)!=0:raise RuntimeError('recipe placement consumer failed')
            result['authority_limits']='One common Client per mode/UI/amount: original recipe placement, one result QUICK_MOVE, actual full grid and main inventory, observed output gain and unchanged empty/held dirt cursor, stale-grid refusal, original table close, selection, disconnect. Read-only native inventory and position independently checked. Gain is received stock increase, not predicted batch count or a guarantee against later native partial/drop behavior.'
            result['result']='passed'
        finally:
            if probe.poll() is None:
                probe.terminate()
                try:probe.wait(timeout=10)
                except subprocess.TimeoutExpired:probe.kill();probe.wait(timeout=5)


def run_recipe_ghost(version, env, rcon, trace, report, probe_log, stderr_log):
    results=report['native_results']['recipe_ghost']={}
    for mode in ('survival','creative'):
      for ui in ('player','table'):
       for amount in ('next','maximum'):
        until(lambda:matched(rcon.command('execute unless entity @a[name=UnifiedProbe]'),'Test passed'))
        result=results[mode+'-'+ui+'-'+amount]={'fixture':{},'records':[]}
        for command in ['setblock 0 65 1 minecraft:air', 'setblock 0 65 2 minecraft:crafting_table' if ui=='table' else 'setblock 0 65 2 minecraft:air']:
            result['fixture'][command]=rcon.command(command)
        probe=subprocess.Popen([str(REPO/'target/debug/examples/common_native_probe')],cwd=REPO,
            env=dict(env,VOXRIG_NATIVE_SCENARIO='recipe-ghost',VOXRIG_NATIVE_RECIPE_MODE=mode,VOXRIG_NATIVE_RECIPE_UI=ui,VOXRIG_NATIVE_RECIPE_AMOUNT=amount),stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=stderr_log,text=True,bufsize=1)
        messages=queue.Queue();reader=threading.Thread(target=pump,args=(probe.stdout,messages,probe_log),daemon=True);reader.start()
        try:
            stage(probe,messages,'b4_ghost_ready',result['records'])
            commands=['gamemode '+mode+' UnifiedProbe','tp UnifiedProbe 0.5 65 0.5 0 0','clear UnifiedProbe']
            for index,item in ((0,'oak_planks'),(2,'dirt')):
                commands.append(f'replaceitem entity UnifiedProbe inventory.{index} minecraft:{item} 1' if version=='1.16.1' else f'item replace entity UnifiedProbe inventory.{index} with minecraft:{item} 1')
            commands.append('recipe give UnifiedProbe *')
            for command in commands:result['fixture'][command]=rcon.command(command)
            boundary=trace.mark()
            done=stage(probe,messages,'b4_ghost_run',result['records'],timeout=120)['value']
            expected={'dirt':1,'oak_button':1}
            def inventory():
                response=rcon.command('data get entity UnifiedProbe Inventory');stacks=outer_snbt_compounds(response)
                if len(stacks)!=len(expected):return None
                for item,count in expected.items():
                    if not any(f'id: "minecraft:{item}"' in s and re.search(rf'(?:Count|count): {count}(?:b)?(?:,|\s|}})',s) for s in stacks):return None
                return response
            result['native_inventory']=until(inventory)
            result['native_position']=rcon.command('data get entity UnifiedProbe Pos')
            if '[0.5d, 65.0d, 0.5d]' not in result['native_position']:raise RuntimeError('recipe placement moved native actor')
            result['operation_frames']=trace.since(boundary)
            requests=[f for f in result['operation_frames'] if f['phase']=='play' and f['direction']=='serverbound' and f['packet_id']==(0x19 if version=='1.16.1' else 0x26)]
            if len(requests)!=3:raise RuntimeError('expected two ghost requests then one ordinary placement')
            if [bytes.fromhex(f['body_hex'])[-1] for f in requests]!=[int(amount=='maximum'),int(amount=='maximum'),0]:
                raise RuntimeError('native recipe amount flags differ')
            responses=[f for f in result['operation_frames'] if f['phase']=='play' and f['direction']=='clientbound' and f['packet_id']==(0x30 if version=='1.16.1' else 0x3d)]
            if len(responses)!=2:raise RuntimeError('expected exactly two original ghost responses')
            if len(done['ghosts'])!=2 or any(g['record']['stage']!='observed_ghost' or not g['record']['send']['dispatched'] for g in done['ghosts']):
                raise RuntimeError('actual conserved ghost completion absent')
            if not done['placement']['send']['dispatched'] or done['placement']['stage']!='observed_placed' or done['placement']['ghost'] is not None:
                raise RuntimeError('historical ghost poisoned ordinary placement')
            trace.expect_disconnect();stage(probe,messages,'b4_ghost_disconnect',result['records'])
            if probe.wait(timeout=15)!=0:raise RuntimeError('recipe placement consumer failed')
            result['authority_limits']='One unchanged common Client per mode/UI/amount: explicit dirt then single-plank input -> two native material-shortage ghosts with actual conserved returns and old-plan rejection -> fresh ordinary button placement -> explicit take/deposit -> empty grid/cursor -> original table close -> disconnect. RCON after baseline only reads exact original inventory and unchanged position. Modern display contains no recipe ID; ghost completion does not assert selected-recipe causation or output manufacture.'
            result['result']='passed'
        finally:
            if probe.poll() is None:
                probe.terminate()
                try:probe.wait(timeout=10)
                except subprocess.TimeoutExpired:probe.kill();probe.wait(timeout=5)


def run_connection_revocation(version, env, rcon, trace, report, probe_log, stderr_log):
    """Observe local revocation and actual peer closure while both clones live."""
    result = report['native_results']['connection_revocation'] = {'records':[]}
    probe = subprocess.Popen([str(REPO/'target/debug/examples/common_native_probe')], cwd=REPO,
        env=dict(env, VOXRIG_NATIVE_SCENARIO='connection-revocation'), stdin=subprocess.PIPE,
        stdout=subprocess.PIPE, stderr=stderr_log, text=True, bufsize=1)
    messages = queue.Queue()
    reader = threading.Thread(target=pump,args=(probe.stdout,messages,probe_log),daemon=True)
    reader.start()
    try:
        ready = stage(probe,messages,'b5_revocation_ready',result['records'])['value']
        result['native_player_before'] = until(lambda: matched(rcon.command('list'), 'UnifiedProbe'))
        selected_id = 0x24 if version == '1.16.1' else 0x34
        def selections():
            return [f for f in trace.frames if f['phase']=='play' and f['direction']=='serverbound' and f['packet_id']==selected_id]
        until(lambda:selections())
        revoked = stage(probe,messages,'b5_revoke',result['records'])['value']
        if revoked['receipt']['connection_id'] != ready['session']['connection_id'] or revoked['receipt']['version'] != ready['session']['version']:
            raise RuntimeError('revocation identity differs from actual baseline')
        def absent():
            value=rcon.command('list')
            return value if 'UnifiedProbe' not in value else None
        result['native_player_after'] = until(absent)
        result['terminal_events'] = until(lambda:[e for e in trace.terminal_events if e['direction']=='serverbound' and e['kind']=='clean_eof'])
        stage(probe,messages,'b5_revocation_observed',result['records'])
        if len(selections()) != 1:
            raise RuntimeError('revoked original/clone hotbar selections reached server')
        result['selection_frame'] = selections()[0]
        if probe.wait(timeout=15) != 0:
            raise RuntimeError('common revocation consumer failed')
        result['authority_limits']='Local irreversible fencing is distinct from observed peer EOF/player absence in this run. No general guarantee of transport closure, cancellation of prior effects or server stillness. Capture/writer stalls and partial writes are checked separately in TCP/stream fixtures.'
        result['result']='passed'
    finally:
        if probe.poll() is None:
            probe.terminate()
            try: probe.wait(timeout=10)
            except subprocess.TimeoutExpired: probe.kill(); probe.wait(timeout=5)


def run_mining_recovery(version, env, rcon, trace, report, probe_log, stderr_log, case, tool_case=None):
    """A3: original mining -> explicit fresh admission -> actual placement.
    After the initial fixture, every RCON operation is read-only.
    """
    group = 'mining_tools' if tool_case else 'mining_recovery'
    block, tool = tool_case if tool_case else ('minecraft:stone', None)
    result = report['native_results'].setdefault(group, {})[case] = {'fixture':{}, 'records':[]}
    tool_env = {'VOXRIG_NATIVE_MINING_TOOL':tool, 'VOXRIG_NATIVE_MINING_BLOCK':block.split('[')[0]} if tool else {}
    for command in ['setblock 0 65 1 minecraft:air', 'setblock 0 65 3 '+block, 'setblock 2 65 0 minecraft:air']:
        result['fixture'][command] = rcon.command(command)
    probe = subprocess.Popen([str(REPO/'target/debug/examples/common_native_probe')], cwd=REPO,
        env=dict(env, VOXRIG_NATIVE_SCENARIO='mining-recovery', VOXRIG_NATIVE_RECOVERY_CASE=case, **tool_env), stdin=subprocess.PIPE,
        stdout=subprocess.PIPE, stderr=stderr_log, text=True, bufsize=1)
    messages = queue.Queue()
    reader = threading.Thread(target=pump,args=(probe.stdout,messages,probe_log),daemon=True)
    reader.start()
    try:
        stage(probe,messages,'mining_ready',result['records'])
        for command in ['tp UnifiedProbe 0.5 65 0.5 0 0','clear UnifiedProbe',
                        ('replaceitem entity UnifiedProbe hotbar.1 minecraft:stone 3' if version=='1.16.1'
                         else 'item replace entity UnifiedProbe hotbar.1 with minecraft:stone 3')]:
            result['fixture'][command] = rcon.command(command)
        if tool:
            command = ('replaceitem entity UnifiedProbe hotbar.0 '+tool if version=='1.16.1' else 'item replace entity UnifiedProbe hotbar.0 with '+tool)
            result['fixture'][command] = rcon.command(command)
        baseline = stage(probe,messages,'mining_baseline',result['records'])['value']
        result['position_before'] = rcon.command('data get entity UnifiedProbe Pos')
        result['inventory_before'] = rcon.command('data get entity UnifiedProbe Inventory')
        mark = trace.mark()
        started = stage(probe,messages,'mining_start',result['records'])['value']
        started_at = time.monotonic()
        pending = case == 'pending'
        if not pending:
            time.sleep(started['estimated_wait_ms']/1000 + 0.25)
        removed = stage(probe,messages,'mining_pending_finish' if pending else 'mining_finish',result['records'])['value']
        original_state = 'minecraft:stone' if pending else 'minecraft:air'
        result['native_target_before_recovery'] = until(lambda:matched(rcon.command('execute if block 0 65 3 '+original_state),'Test passed'))
        trace.expect_disconnect()
        fresh = stage(probe,messages,'mining_recover',result['records'])['value']
        placed = stage(probe,messages,'recovery_place',result['records'])['value']
        placed_cell = '2 65 0' if pending else '0 65 3'
        result['native_placed_after_recovery'] = until(lambda:matched(rcon.command('execute if block '+placed_cell+' minecraft:stone'),'Test passed'))
        if pending:
            time.sleep(max(0, started_at + started['estimated_wait_ms']/1000 + 0.5 - time.monotonic()))
            result['original_stone_after_old_delayed_break_deadline'] = rcon.command('execute if block 0 65 3 minecraft:stone')
            if 'Test passed' not in result['original_stone_after_old_delayed_break_deadline']:
                raise RuntimeError('old delayed mining continued after fresh recovery')
        def remaining():
            raw = rcon.command('data get entity UnifiedProbe Inventory')
            stacks = outer_snbt_compounds(raw)
            placement = [s for s in stacks if re.search(r'Slot: 1b(?:,|\s|})',s) and 'id: "minecraft:stone"' in s and re.search(r'(?:Count: 2b|count: 2)(?:,|\s|})',s)]
            held = [s for s in stacks if re.search(r'Slot: 0b(?:,|\s|})',s) and ('id: "'+tool+'"') in s] if tool else []
            return raw if len(placement)==1 and len(stacks)==(2 if tool else 1) and (not tool or len(held)==1) else None
        result['inventory_after'] = until(remaining)
        if tool:
            held = next(s for s in outer_snbt_compounds(result['inventory_after']) if re.search(r'Slot: 0b(?:,|\s|})',s))
            if not re.search(r'(?:Damage|"minecraft:damage"): 1(?:,|\s|})', held):
                raise RuntimeError('native tool durability did not change once: '+held)
            result['native_worn_tool'] = held
            if started['estimate']['tool_speed'] <= 1 or not started['estimate']['harvestable']:
                raise RuntimeError('common native-default tool estimate not retained')
        result['position_after'] = rcon.command('data get entity UnifiedProbe Pos')
        if result['position_after'] != result['position_before']:
            raise RuntimeError('recovery changed native position')
        source_stamp = baseline['session']
        fresh_stamp = fresh['identity']['session']
        if started['id']['session']!=source_stamp or removed['id']['session']!=source_stamp or fresh_stamp['connection_id']==source_stamp['connection_id']:
            raise RuntimeError('invalid mining recovery source/fresh identity')
        if fresh['target']['name']!=original_state or fresh['original']['continuation_validated'] or fresh['original']['recovery_attempt']['method']!='same_profile_login':
            raise RuntimeError('recovery source history or target differs')
        if placed['player']['session']!=fresh_stamp or placed['placement']['id']['session']!=fresh_stamp or placed['original']['id']['session']!=source_stamp or placed['original']['continuation_validated']:
            raise RuntimeError('placement reused original mining authority')
        frames = trace.since(mark)
        result['operation_frames'] = frames
        play = [f for f in frames if f['direction']=='serverbound' and f['phase']=='play']
        mining_id = 0x1b if version=='1.16.1' else 0x28
        placement_id = 0x2d if version=='1.16.1' else 0x3f
        mines = [f for f in play if f['packet_id']==mining_id]
        placements = [f for f in play if f['packet_id']==placement_id]
        # Packet IDs are selected only in the native wire oracle, not the consumer.
        if len(mines)!=2 or len(placements)!=1 or len({f['connection'] for f in mines})!=1 or placements[0]['connection']==mines[0]['connection']:
            raise RuntimeError('expected one source START/FINISH and one fresh placement: '+repr([(f['connection'],f['packet_id']) for f in play]))
        all_frames = trace.since(0)
        profiles = []
        for connection in (mines[0]['connection'], placements[0]['connection']):
            packets = [f for f in all_frames if f['connection']==connection and f['direction']=='clientbound' and f['phase']=='login' and f['packet_id']==2]
            if len(packets)!=1: raise RuntimeError('expected exactly one actual LOGIN_SUCCESS per mining/recovery connection')
            raw = bytes.fromhex(packets[0]['body_hex'])
            length,offset = PacketTraceProxy.varint(raw[16:]); offset += 16
            if raw[offset:offset+length] != b'UnifiedProbe': raise RuntimeError('native login profile name differs')
            profiles.append({'connection':connection,'uuid_hex':raw[:16].hex(),'name':'UnifiedProbe'})
        if profiles[0]['uuid_hex']!=profiles[1]['uuid_hex'] or bytes(fresh['identity']['uuid']).hex()!=profiles[1]['uuid_hex']:
            raise RuntimeError('fresh common identity differs from original received native UUID')
        result['original_login_profiles'] = profiles
        result['authority_limits'] = 'Direct unmodified vanilla, exclusively owned offline profile; original closed and still blocked; same received UUID/name, new play/join/loaded position/inventory and dry standing admission; explicit once-only login then a new placement. Air and native ACK alone never release the source. No game-state edits after baseline, inherited authority or automatic retries.'
        trace.expect_disconnect()
        stage(probe,messages,'recovery_disconnect',result['records'])
        probe.wait(timeout=10); reader.join(timeout=2)
        if probe.returncode!=0 or reader.is_alive(): raise RuntimeError('A3 probe failed at disconnect')
        result['result'] = 'passed'
    finally:
        if probe.poll() is None:
            probe.terminate()
            try: probe.wait(timeout=10)
            except subprocess.TimeoutExpired: probe.kill(); probe.wait(timeout=5)


def run_recording_scene(version, env, rcon, trace, report, probe_log, stderr_log, folder):
    """Actual original receive bytes, detached forecast, offline replay; one JVM."""
    result=report['native_results']['recording_scene']={'fixture':{},'records':[]}
    path=folder / 'common-packet-recording.json'
    probe=subprocess.Popen([str(REPO / 'target/debug/examples/common_native_probe')],cwd=REPO,
        env=dict(env,VOXRIG_NATIVE_SCENARIO='recording-scene',VOXRIG_NATIVE_RECORDING_PATH=str(path)),
        stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=stderr_log,text=True,bufsize=1)
    messages=queue.Queue()
    reader=threading.Thread(target=pump,args=(probe.stdout,messages,probe_log),daemon=True);reader.start()
    try:
        stage(probe,messages,'a4_ready',result['records'])
        item=('minecraft:oak_planks{VoxrigA4:{marker:"recorded",values:[I;1,2,3]}}' if version=='1.16.1'
            else 'minecraft:oak_planks[minecraft:custom_data={VoxrigA4:{marker:"recorded",values:[I;1,2,3]}}]')
        for command in ('gamemode survival UnifiedProbe','clear UnifiedProbe','tp UnifiedProbe 0.5 65 0.5 0 0','give UnifiedProbe '+item+' 3'):
            response=rcon.command(command);result['fixture'][command]=response
            if any(word in response for word in ('Incorrect argument','Unknown or incomplete')):raise RuntimeError('A4 fixture rejected: '+response)
        probe.stdin.write('a4_capture\n');probe.stdin.flush()
        # Consumer completion uses a distinct stage while the submitted command
        # remains explicit, like the other retained-operation scenarios.
        captured=stage(probe,messages,'a4_captured',result['records'])['value']
        result['captured']=captured
        saved=json.loads(path.read_text())
        received=[f for f in trace.since(0) if f['direction']=='clientbound' and f['phase'] in ('configuration','play')]
        if not saved['complete'] or saved['after_sequence']!=0 or saved['through_sequence']!=len(saved['records']):raise RuntimeError('A4 incomplete trace')
        for index,record in enumerate(saved['records']):
            frame=received[index];body=bytes(record['payload'])
            if record['sequence']!=index+1 or (record['phase'],record['packet_id'],len(body),hashlib.sha256(body).hexdigest())!=(frame['phase'],frame['packet_id'],frame['body_length'],frame['body_sha256']):
                raise RuntimeError('A4 recording differs from original native receive frame')
        result['recording']={'sha256':hashlib.sha256(path.read_bytes()).hexdigest(),'records':len(saved['records']),'bytes':sum(len(r['payload']) for r in saved['records']),'exact_original_frames_verified':len(saved['records'])}
        result['native_inventory']=until(lambda: matched(rcon.command('data get entity UnifiedProbe Inventory'), 'VoxrigA4'))
        stacks=outer_snbt_compounds(result['native_inventory'])
        if len(stacks)!=1 or 'id: "minecraft:oak_planks"' not in stacks[0] or not re.search(r'(?:Count|count): 3(?:b)?(?:,|\s|})',stacks[0]) or 'recorded' not in stacks[0]:
            raise RuntimeError('A4 original native inventory/item data differs')
        result['native_position']=rcon.command('data get entity UnifiedProbe Pos')
        if not re.search(r'0\.5d, 65\.0d, 0\.5d',result['native_position']):raise RuntimeError('A4 read-only scene moved native player')
        result['native_target']=rcon.command('execute if block 0 65 1 minecraft:stone')
        if 'Test passed' not in result['native_target']:raise RuntimeError('A4 replay target differs from native block')
        # Change actual geometry after capture; old scene must retain its snapshot.
        result['live_update_fixture']=rcon.command('setblock 1 65 0 minecraft:stone')
        trace.expect_disconnect()
        result['detached']=stage(probe,messages,'a4_detached',result['records'])['value']
        probe.wait(timeout=10);reader.join(timeout=2)
        if probe.returncode!=0 or reader.is_alive():raise RuntimeError('A4 consumer failed shutdown')
        frames=trace.since(0)
        # This scenario never invokes a game mutation; only native session/motion
        # heartbeat and mandatory acknowledgements may appear on the wire.
        result['serverbound_packet_ids']=sorted({f['packet_id'] for f in frames if f['phase']=='play' and f['direction']=='serverbound'})
        allowed={0x00,0x05,0x0b,0x10,0x12,0x13,0x14,0x15} if version=='1.16.1' else {0x00,0x0a,0x0c,0x0d,0x15,0x1b,0x1d,0x1e,0x1f,0x20,0x2b,0x2c}
        if set(result['serverbound_packet_ids'])-allowed:raise RuntimeError('A4 read-only capture/replay emitted a game mutation')
        result['authority_limits']='Original payloads/ordinals verified against a transparent compressed-byte proxy. Received pose, health, complete player inventory data/provenance and one native block match live observation. Immutable dry-cube scene matches native live forecast and survives an actual world update/source closure. Replay owns no connection or execution IDs; full protocol reconstruction, edits/chaining and authenticated saved histories are not claimed.'
        result['result']='passed'
    finally:
        if probe.poll() is None:
            probe.terminate()
            try:probe.wait(timeout=10)
            except subprocess.TimeoutExpired:probe.kill();probe.wait(timeout=5)


def verify_scoreboard_frames(version, observations, identities, trace):
    """Bind public per-value ordinals to actual original native peer frames."""
    all_frames=trace.since(0);peers={}
    for frame in all_frames:
        if frame['direction']=='clientbound' and frame['phase']=='login' and frame['packet_id']==2:
            body=bytes.fromhex(frame['body_hex']);length,size=PacketTraceProxy.varint(body[16:]);name=body[16+size:16+size+length].decode()
            peers[name]=frame['connection']
    verified=[]
    def integer(n):
        n &= 0xffffffff;out=bytearray()
        while n>=128:out.append((n&127)|128);n>>=7
        out.append(n);return bytes(out)
    def string(s):
        data=s.encode();return integer(len(data))+data
    def text(t):
        return string(t['json']) if t['kind']=='legacy_json' else bytes(t['bytes'])
    def fmt(v):
        if v is None:return b'\x00'
        kinds={'blank':0,'styled':1,'fixed':2};out=b'\x01'+integer(kinds[v['kind']])
        if v['kind']=='styled':out+=bytes(v['bytes'])
        if v['kind']=='fixed':out+=text(v['text'])
        return out
    for observation,key in zip(observations,('primary','peer')):
        identity=identities[key]
        if observation['session']!=identity['session']:raise RuntimeError('scoreboard captured from another managed connection/world')
        frames=[f for f in all_frames if f['connection']==peers[identity['name']] and f['direction']=='clientbound' and f['phase'] in ('configuration','play')]
        def verify(value,kind,payload):
            source=value['source']
            if source['kind']!='received':raise RuntimeError('scoreboard invented a receive origin')
            frame=frames[source['sequence']-1]
            if frame['packet_id']!=kind or len(payload)!=frame['body_length'] or hashlib.sha256(payload).hexdigest()!=frame['body_sha256']:
                raise RuntimeError('scoreboard field differs from the original native packet')
            verified.append({k:frame[k] for k in ('connection','ordinal','packet_id','body_sha256')})
        for declaration in observation['objectives']:
            o=declaration['value'];source=frames[declaration['source']['sequence']-1];body=bytes.fromhex(source['body_hex']);_,size=PacketTraceProxy.varint(body);action=body[size+len(o['name'].encode())]
            payload=string(o['name'])+bytes([action])+text(o['display'])+integer({'integer':0,'hearts':1}[o['render_type']])
            if version!='1.16.1':payload+=fmt(o['number_format'])
            verify(declaration,0x4a if version=='1.16.1' else 0x68,payload)
        for slot,declaration in observation['displays'].items():verify(declaration,0x43 if version=='1.16.1' else 0x60,integer(int(slot))+string(declaration['value']))
        for entry in observation['scores']:
            e=entry['value'];payload=string(e['owner'])
            if version=='1.16.1':payload+=b'\x00'
            payload+=string(e['objective'])+integer(e['value'])
            if version!='1.16.1':payload+=(b'\x00' if e['display'] is None else b'\x01'+text(e['display']))+fmt(e['number_format'])
            verify(entry,0x4d if version=='1.16.1' else 0x6c,payload)
    return verified,peers


def verify_boss_bar_frames(version, observations, identities, trace, peers):
    """Compare each receipt to independently decoded original native frame fields."""
    all_frames=trace.since(0);verified=[]
    def decode(frame):
        raw=bytes.fromhex(frame['body_hex']);cursor=0
        def take(n):
            nonlocal cursor
            if n<0 or cursor+n>len(raw):raise RuntimeError('truncated original boss frame')
            value=raw[cursor:cursor+n];cursor+=n;return value
        def varint():
            nonlocal cursor
            value,size=PacketTraceProxy.varint(raw[cursor:]);cursor+=size;return value
        def string():return take(varint()).decode()
        def nbt_payload(tag,depth=0):
            if depth>64:raise RuntimeError('native boss component depth exceeded')
            if tag in (1,2,3,4,5,6):take({1:1,2:2,3:4,4:8,5:4,6:8}[tag])
            elif tag in (7,11,12):take(int.from_bytes(take(4),'big',signed=True)*{7:1,11:4,12:8}[tag])
            elif tag==8:take(int.from_bytes(take(2),'big'))
            elif tag==9:
                child=take(1)[0];count=int.from_bytes(take(4),'big',signed=True)
                if not 0<=count<=1048576:raise RuntimeError('invalid native component list')
                for _ in range(count):nbt_payload(child,depth+1)
            elif tag==10:
                while True:
                    child=take(1)[0]
                    if child==0:break
                    take(int.from_bytes(take(2),'big'));nbt_payload(child,depth+1)
            else:raise RuntimeError('unsupported original text NBT tag')
        def text():
            if version=='1.16.1':return {'kind':'legacy_json','json':string()}
            start=cursor;tag=take(1)[0];nbt_payload(tag);return {'kind':'native_nbt','bytes':list(raw[start:cursor])}
        uuid=list(take(16));operation=varint();fields={}
        if operation in (0,3):fields['title']=text()
        if operation in (0,2):fields['progress']=struct.unpack('>f',take(4))[0]
        if operation in (0,4):
            color=varint();overlay=varint()
            fields['color']=('pink','blue','red','green','yellow','purple','white')[color]
            fields['overlay']=('progress','notched6','notched10','notched12','notched20')[overlay]
        if operation in (0,5):fields['flags']={'raw':take(1)[0]}
        if cursor!=len(raw) or operation not in range(6):raise RuntimeError('unexpected original boss frame fields')
        return uuid,operation,fields
    for observation,key in zip(observations,('primary','peer')):
        identity=identities[key]
        if observation['session']!=identity['session']:raise RuntimeError('boss bars from another managed session')
        frames=[f for f in all_frames if f['connection']==peers[identity['name']] and f['direction']=='clientbound' and f['phase'] in ('configuration','play')]
        def source(sequence):
            frame=frames[sequence-1]
            if frame['packet_id']!=(0x0c if version=='1.16.1' else 0x09):raise RuntimeError('bar ordinal belongs to another packet')
            return frame
        for bar in observation['bars']:
            for name in ('title','progress','color','overlay','flags'):
                receipt=bar[name]
                if receipt['source']['kind']!='received':raise RuntimeError('bar field has no native source')
                frame=source(receipt['source']['sequence']);uuid,op,fields=decode(frame)
                if uuid!=bar['uuid'] or fields.get(name)!=receipt['value']:raise RuntimeError('bar field differs from original packet')
                verified.append(dict(field=name,**{k:frame[k] for k in ('connection','ordinal','packet_id','body_sha256')}))
        latest=source(observation['last_update_sequence']);_,operation,_=decode(latest)
        if not observation['bars'] and operation!=1:raise RuntimeError('removed bar has no actual REMOVE source')
        verified.append(dict(event=operation,**{k:latest[k] for k in ('connection','ordinal','packet_id','body_sha256')}))
    return verified


def run_manager_ui(version,env,rcon,trace,report,probe_log,stderr_log):
    result=report['native_results']['manager_ui']={'fixture':{},'records':[]}
    probe=subprocess.Popen([str(REPO/'target/debug/examples/common_native_probe')],cwd=REPO,env=dict(env,VOXRIG_NATIVE_SCENARIO='manager-ui'),stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=stderr_log,text=True,bufsize=1)
    messages=queue.Queue();reader=threading.Thread(target=pump,args=(probe.stdout,messages,probe_log),daemon=True);reader.start()
    try:
        ready=stage(probe,messages,'a5_ui_ready',result['records'])['value'];result['managed']=ready
        result['native_players']=until(lambda:matched(rcon.command('list'),r'2 of'))
        for command in ('tp UnifiedProbe 0.5 65 0.5','tp ManagedPeer 3.5 65 0.5','scoreboard objectives add voxrigA5 dummy {"text":"A5","color":"gold"}','scoreboard objectives setdisplay sidebar voxrigA5','scoreboard players set UnifiedProbe voxrigA5 7','scoreboard players set ManagedPeer voxrigA5 3'):
            response=rcon.command(command);result['fixture'][command]=response
            if any(word in response for word in ('Incorrect argument','Unknown or incomplete')):raise RuntimeError('manager/UI fixture rejected: '+response)
        result['baseline']=stage(probe,messages,'a5_ui_baseline',result['records'])['value']
        verified,peers=verify_scoreboard_frames(version,result['baseline'],ready['identities'],trace);result['original_fields']=verified
        result['update_command']=rcon.command('scoreboard players set UnifiedProbe voxrigA5 9')
        result['updated']=stage(probe,messages,'a5_ui_updated',result['records'])['value']
        verified,_=verify_scoreboard_frames(version,result['updated'],ready['identities'],trace);result['original_fields']+=verified
        result['native_score']=rcon.command('scoreboard players get UnifiedProbe voxrigA5')
        if not re.search(r'has 9',result['native_score']):raise RuntimeError('native scoreboard value differs')
        result['reset_command']=rcon.command('scoreboard players reset UnifiedProbe')
        result['reset']=stage(probe,messages,'a5_ui_reset',result['records'])['value']
        result['remove_command']=rcon.command('scoreboard objectives remove voxrigA5')
        result['removed']=stage(probe,messages,'a5_ui_removed',result['records'])['value']
        bars=result['boss_bars']={'commands':{},'stages':{}}
        for command in ('gamemode survival UnifiedProbe','gamemode creative ManagedPeer','bossbar add voxrigb6 {"text":"B6"}','bossbar set voxrigb6 max 100','bossbar set voxrigb6 value 50','bossbar set voxrigb6 color red','bossbar set voxrigb6 style notched_20','bossbar set voxrigb6 players @a'):
            response=rcon.command(command);bars['commands'][command]=response
            if any(word in response for word in ('Incorrect argument','Unknown or incomplete','Expected whitespace')):raise RuntimeError('bossbar fixture rejected: '+response)
        bars['native_players']=rcon.command('bossbar get voxrigb6 players')
        bars['stages']['added']=stage(probe,messages,'b6_bars_added',result['records'])['value']
        bars['original_fields']=verify_boss_bar_frames(version,bars['stages']['added']['bars'],ready['identities'],trace,peers)
        for command in ('bossbar set voxrigb6 value 75','bossbar set voxrigb6 name {"text":"UpdatedB6"}','bossbar set voxrigb6 color green','bossbar set voxrigb6 style notched_6'):
            bars['commands'][command]=rcon.command(command)
        bars['stages']['updated']=stage(probe,messages,'b6_bars_updated',result['records'])['value']
        bars['original_fields']+=verify_boss_bar_frames(version,bars['stages']['updated']['bars'],ready['identities'],trace,peers)
        bars['native_value']=rcon.command('bossbar get voxrigb6 value')
        if not re.search(r'75',bars['native_value']):raise RuntimeError('native bar progress differs')
        for old,new in zip(bars['stages']['added']['bars'],bars['stages']['updated']['bars']):
            a,b=old['bars'][0],new['bars'][0]
            if a['uuid']!=b['uuid'] or a['flags']!=b['flags'] or a['progress']['source']==b['progress']['source'] or a['title']['source']==b['title']['source']:
                raise RuntimeError('partial bar update lost unchanged flags or fresh fields')
        bars['commands']['remove']=rcon.command('bossbar remove voxrigb6')
        bars['stages']['removed']=stage(probe,messages,'b6_bars_removed',result['records'])['value']
        bars['original_fields']+=verify_boss_bar_frames(version,bars['stages']['removed']['bars'],ready['identities'],trace,peers)
        bars['native_remaining']=rcon.command('bossbar list')
        if 'no custom bossbars' not in bars['native_remaining'].lower():raise RuntimeError('native bar still exists')
        for index,mode in enumerate(('survival','creative')):
            if bars['stages']['updated']['players'][index]['game_mode']!=mode:raise RuntimeError('UI modes differ from requested fixture')
        bars['result']='passed'
        # Scope closure to both actual managed transports, not a global flag.
        for connection in peers.values():trace.expect_disconnect(connection)
        result['shutdown']=stage(probe,messages,'a5_manager_shutdown',result['records'])['value']
        probe.wait(timeout=10);reader.join(timeout=2)
        if probe.returncode!=0 or reader.is_alive():raise RuntimeError('manager/UI probe failed shutdown')
        result['native_after_shutdown']=until(lambda:matched(rcon.command('list'),r'0 of'))
        logins=[f for f in trace.since(0) if f['direction']=='clientbound' and f['phase']=='login' and f['packet_id']==2]
        if len(logins)!=2:raise RuntimeError('duplicate admission or terminal shutdown performed another login')
        result['actual_logins']=len(logins);result['result']='passed'
        result['authority_limits']='Two actual named Client connections per version, coherent received scoreboard with exact original field payload hashes/ordinals, independent native score and player-list checks, fresh update/owner reset/objective removal, terminal manager shutdown closes externally held clones and refuses further login. Mixed-version registry identity separation and cancelled pending login closure use lightweight TCP fixtures; A5 representative workflows are complete; boss-bar add/update/remove also match original wire fields on both managed connections. Rendering, other UI and wider B6 remain required.'
    finally:
        if probe.poll() is None:
            probe.terminate()
            try:probe.wait(timeout=10)
            except subprocess.TimeoutExpired:probe.kill();probe.wait(timeout=5)


def run(version, accept_eula, runtime_root=None, runtime_inputs=None, scenario="full"):
    if not accept_eula:
        raise RuntimeError("pass --accept-eula when authorized to run the official server")
    jar, source = download(version)
    run_id = "trial-" + version + "-" + uuid.uuid4().hex[:8]
    retained = ROOT / run_id
    folder = (runtime_root or ROOT) / run_id
    folder.mkdir(parents=True)
    port, rcon_port = free_port(), free_port()
    while port == rcon_port:
        rcon_port = free_port()
    password = secrets.token_hex(24)
    (folder / "eula.txt").write_text("eula=true\n")
    generator_settings = json.dumps({
        "biome": "minecraft:plains",
        "layers": [
            {"block": "minecraft:bedrock", "height": 1},
            {"block": "minecraft:dirt", "height": 2},
            {"block": "minecraft:grass_block", "height": 1},
        ],
        "structure_overrides": [],
    }) if version == "1.21.11" else ""
    properties = f"""server-ip=127.0.0.1
server-port={port}
level-name=world
level-type=flat
generator-settings={generator_settings}
level-seed=1234
online-mode=false
gamemode={"survival" if scenario in ("mining-recovery","mining-tools","recording-scene") else "creative"}
force-gamemode=true
spawn-protection=0
view-distance=2
simulation-distance=2
max-players=4
max-tick-time=60000
enable-rcon=true
rcon.port={rcon_port}
rcon.password={password}
broadcast-rcon-to-ops=false
enable-status=false
sync-chunk-writes=false
network-compression-threshold=256
"""
    (folder / "server.properties").write_text(properties)
    os.chmod(folder / "server.properties", 0o600)
    server_log = (folder / "server.log").open("w")
    stderr_log = (folder / "probe-stderr.log").open("w")
    probe_log = (folder / "probe.jsonl").open("w")
    server = subprocess.Popen(["java", "-XX:ActiveProcessorCount=1", "-Xms256M", "-Xmx1024M", "-jar", str(jar), "nogui"], cwd=folder, stdin=subprocess.PIPE, stdout=server_log, stderr=subprocess.STDOUT, text=True)
    probe, rcon = None, None
    trace = PacketTraceProxy(port, version, folder / "packet-trace.jsonl")
    report = {"version":version, "run_id":folder.name, "source":source, "server_memory_limit":"1024M", "sync_chunk_writes":False, "result":"running", "scenario_result":"running", "native_results":{}, "client_records":[]}
    report["runtime_parent"] = str(folder.parent)
    report["runtime_inputs"] = runtime_inputs
    (folder / "process.json").write_text(json.dumps({"server_pid":server.pid, "version":version, "started":time.time()}) + "\n")
    print(version, "server", server.pid, "log", folder, flush=True)
    try:
        def connect():
            if server.poll() is not None:
                raise RuntimeError("server stopped: " + (folder / "server.log").read_text()[-3500:])
            try:
                return Rcon(rcon_port, password)
            except (OSError, TimeoutError):
                return None
        rcon = until(connect, 120)
        report["force_load"] = rcon.command("forceload add -8 -8 8 8")
        def loaded():
            rcon.command("setblock 0 64 0 minecraft:stone")
            return matched(rcon.command("execute if block 0 64 0 minecraft:stone"), "Test passed")
        report["force_load_result"] = until(loaded, 30)
        # 1.21.11 renamed these server commands. This is fixture bootstrap;
        # the common Client consumer below never selects a version-specific API.
        rules = (("spawnRadius", "doMobSpawning", "doDaylightCycle") if version == "1.16.1"
                 else ("minecraft:respawn_radius", "minecraft:spawn_mobs", "minecraft:advance_time"))
        setup = [f"gamerule {rules[0]} 0", f"gamerule {rules[1]} false", f"gamerule {rules[2]} false", "weather clear", "fill -8 64 -8 8 64 8 minecraft:stone", "fill -8 65 -8 8 69 8 minecraft:air", "setblock 0 65 1 minecraft:stone", "setworldspawn 0 65 0"]
        report["fixture_responses"] = {}
        for command in setup:
            response = rcon.command(command)
            report["fixture_responses"][command] = response
            if any(text in response for text in ("Incorrect argument", "Unknown or incomplete", "not loaded")):
                raise RuntimeError("fixture command rejected: " + command + ": " + response)
        for check in ["execute if block 0 65 1 minecraft:stone", "execute if block 1 65 0 minecraft:air"]:
            report.setdefault("fixture_verification", {})[check] = until(lambda: matched(rcon.command(check), "Test passed"))
        env = dict(os.environ, VOXRIG_MINECRAFT_VERSION=version, VOXRIG_PORT=str(trace.port))
        if scenario == "recipe-result-transfer":
            run_recipe_result_transfer(version,env,rcon,trace,report,probe_log,stderr_log)
            report["scenario_result"]="passed"
            return retained
        if scenario == "recipe-result-merge":
            run_recipe_result_merge(version,env,rcon,trace,report,probe_log,stderr_log)
            report["scenario_result"]="passed"
            return retained
        if scenario == "recipe-ghost":
            run_recipe_ghost(version,env,rcon,trace,report,probe_log,stderr_log)
            report["scenario_result"]="passed"
            return retained
        if scenario == "recipe-placement":
            run_recipe_placement(version,env,rcon,trace,report,probe_log,stderr_log)
            report["scenario_result"]="passed"
            return retained
        if scenario == "connection-revocation":
            run_connection_revocation(version,env,rcon,trace,report,probe_log,stderr_log)
            report["scenario_result"]="passed"
            return retained
        if scenario in ("dry-terrain", "creative-flight", "creative-landing"):
            run_dry_terrain(version,env,rcon,trace,report,probe_log,stderr_log,flight=scenario in ("creative-flight","creative-landing"),landing=scenario=="creative-landing")
            report["scenario_result"]="passed"
            return retained
        if scenario in ("vehicle", "vehicle-control"):
            run_vehicle(version,env,rcon,trace,report,probe_log,stderr_log,controlling=scenario=="vehicle-control")
            report["scenario_result"]="passed"
            return retained
        if scenario == "furnace":
            run_furnace(version,env,rcon,trace,report,probe_log,stderr_log)
            report["scenario_result"]="passed"
            return retained
        if scenario == "manager-ui":
            run_manager_ui(version,env,rcon,trace,report,probe_log,stderr_log)
            report["scenario_result"]="passed"
            return retained
        if scenario == "recording-scene":
            run_recording_scene(version, env, rcon, trace, report, probe_log, stderr_log, folder)
            report["scenario_result"] = "passed"
            return retained
        if scenario == "mining-tools":
            for case, block, tool in (
                ("pickaxe-stone", "minecraft:stone", "minecraft:iron_pickaxe"),
                ("shovel-dirt", "minecraft:dirt", "minecraft:iron_shovel"),
                ("axe-planks", "minecraft:oak_planks", "minecraft:iron_axe"),
                ("pickaxe-double-slab", "minecraft:stone_slab[type=double,waterlogged=false]", "minecraft:wooden_pickaxe"),
            ):
                run_mining_recovery(version, env, rcon, trace, report, probe_log, stderr_log, case, (block, tool))
            report["scenario_result"] = "passed"
            return retained
        if scenario == "mining-recovery":
            for recovery_case in ("completed", "pending"):
                run_mining_recovery(version, env, rcon, trace, report, probe_log, stderr_log, recovery_case)
            report["scenario_result"] = "passed"
            return retained
        if scenario == "equipment-entity":
            run_equipment_entity(version, env, rcon, trace, report, probe_log, stderr_log)
            report["scenario_result"] = "passed"
            return retained
        if scenario == "basic-workflow":
            run_basic_workflow(version, env, rcon, trace, report, probe_log, stderr_log)
            report["scenario_result"] = "passed"
            return retained
        probe = subprocess.Popen([str(REPO / "target/debug/examples/common_native_probe")], cwd=REPO, env=env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=stderr_log, text=True, bufsize=1)
        messages = queue.Queue()
        thread = threading.Thread(target=pump, args=(probe.stdout, messages, probe_log), daemon=True)
        thread.start()
        stage(probe, messages, "ready", report["client_records"])
        report["teleport"] = rcon.command("tp UnifiedProbe 0.5 65 0.5 0 0")
        stage(probe, messages, "baseline", report["client_records"])
        stage(probe, messages, "inventory", report["client_records"])
        def inventory():
            response = rcon.command("data get entity UnifiedProbe Inventory")
            if 'id: "minecraft:stone"' in response and "Slot: 1b" in response and re.search(r"(?:Count|count): 3(?:b)?(?:,|\s|})", response):
                return response
        report["native_results"]["inventory"] = until(inventory)
        report["native_results"]["selected_slot"] = until(lambda: matched(rcon.command("data get entity UnifiedProbe SelectedItemSlot"), r": 1$"))
        stage(probe, messages, "flight", report["client_records"])
        def position():
            response = rcon.command("data get entity UnifiedProbe Pos")
            if re.search(r"\[0\.5d, 66\.0d, 0\.5d\]", response):
                return response
        report["native_results"]["flight"] = until(position)
        stage(probe, messages, "break", report["client_records"])
        report["native_results"]["break"] = until(lambda: matched(rcon.command("execute if block 0 65 1 minecraft:air"), "Test passed"))
        stage(probe, messages, "place", report["client_records"])
        report["native_results"]["place"] = until(lambda: matched(rcon.command("execute if block 1 65 0 minecraft:stone"), "Test passed"))
        report["mode_change"] = rcon.command("gamemode survival UnifiedProbe")
        stage(probe, messages, "survival_guard", report["client_records"])
        forbidden = rcon.command("data get entity UnifiedProbe Inventory")
        if 'id: "minecraft:diamond"' in forbidden:
            raise RuntimeError("forbidden creative write affected server inventory")
        report["native_results"]["survival_guard"] = forbidden
        # Creative writes need not echo to their sender. Force a real inventory
        # update before requiring a settled survival capture; RCON verification
        # above never substitutes for a received slot in the Client.
        report["preview_inventory_reset"] = rcon.command("clear UnifiedProbe")
        report["preview_teleport"] = rcon.command("tp UnifiedProbe 0.5 65 0.5 0 0")
        stationary = until(lambda: matched(rcon.command("data get entity UnifiedProbe Pos"), r"\[0\.5d, 65\.0d, 0\.5d\]"))
        stage(probe, messages, "survival_preview", report["client_records"])
        after_preview = rcon.command("data get entity UnifiedProbe Pos")
        if after_preview != stationary:
            raise RuntimeError("read-only preview changed the native stationary position")
        report["native_results"]["preview_position_before"] = stationary
        report["native_results"]["preview_position_after"] = after_preview
        target = stage(probe, messages, "survival_target", report["client_records"])["value"]
        target_position = target["hit"]["position"]
        if target_position != [1, 65, 0]:
            raise RuntimeError("outline selected unexpected native fixture target")
        target_check = until(lambda: matched(rcon.command("execute if block 1 65 0 minecraft:stone"), "Test passed"))
        after_target = rcon.command("data get entity UnifiedProbe Pos")
        if after_target != stationary:
            raise RuntimeError("read-only targeting changed native stationary position")
        report["native_results"]["survival_target"] = {
            "position": target_position, "state": target["hit"]["state"],
            "face": target["hit"]["face"], "point": target["hit"]["point"],
            "native_block": target_check, "position_before": stationary,
            "position_after": after_target,
        }
        positions = []
        def sample_motion():
            raw = rcon.command("data get entity UnifiedProbe Pos")
            numbers = re.findall(r"(-?\d+(?:\.\d+)?(?:[Ee][+-]?\d+)?)d", raw)
            if len(numbers) != 3:
                raise RuntimeError("native motion position could not be parsed: " + raw)
            positions.append({"position": [float(value) for value in numbers], "response": raw})
        completed = stage(probe, messages, "survival_motion", report["client_records"], poll=sample_motion)["value"]
        sample_motion()
        expected = completed["preview"]["frames"][-1]["position"]
        actual = positions[-1]["position"]
        if not all(abs(a-b) < 1e-7 for a,b in zip(actual, expected)):
            raise RuntimeError(f"native endpoint disagrees with dispatch: {actual} != {expected}")
        if max(sample["position"][1] for sample in positions) < 66.0:
            raise RuntimeError("native jump trajectory was not observed")
        if all(abs(actual[axis] - 0.5) < 0.1 for axis in (0,2)):
            raise RuntimeError("native horizontal walking displacement was not observed")
        report["native_results"]["survival_motion"] = {"samples": positions, "predicted_endpoint": expected, "native_endpoint": actual}
        trace.expect_disconnect()
        stage(probe, messages, "disconnect", report["client_records"])
        probe.wait(timeout=10)
        if probe.returncode != 0:
            raise RuntimeError("probe failed after disconnect")
        thread.join(timeout=2)
        if thread.is_alive():
            raise RuntimeError("first probe reader did not finish")
        # Mining retains an unresolved source; run it on a new connection after
        # the motion client has ended. The Rust consumer has no version branches.
        until(lambda: matched(rcon.command("execute unless entity @a[name=UnifiedProbe]"), "Test passed"))
        mining_fixture = rcon.command("setblock 0 65 3 minecraft:stone")
        if "Changed the block" not in mining_fixture:
            raise RuntimeError("native mining fixture rejected: " + mining_fixture)
        report["mining_fixture"] = mining_fixture
        report["mining_records"] = []
        mining_env = dict(env, VOXRIG_NATIVE_SCENARIO="mining")
        probe = subprocess.Popen([str(REPO / "target/debug/examples/common_native_probe")], cwd=REPO, env=mining_env, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=stderr_log, text=True, bufsize=1)
        messages = queue.Queue()
        thread = threading.Thread(target=pump, args=(probe.stdout, messages, probe_log), daemon=True)
        thread.start()
        stage(probe, messages, "mining_ready", report["mining_records"])
        report["mining_mode_change"] = rcon.command("gamemode survival UnifiedProbe")
        report["mining_teleport"] = rcon.command("tp UnifiedProbe 0.5 65 0.5 0 0")
        report["mining_clear_inventory"] = rcon.command("clear UnifiedProbe")
        stage(probe, messages, "mining_baseline", report["mining_records"])
        mining_position = rcon.command("data get entity UnifiedProbe Pos")
        started = stage(probe, messages, "mining_start", report["mining_records"])["value"]
        time.sleep(started["estimated_wait_ms"] / 1000.0 + 0.2)
        completed = stage(probe, messages, "mining_finish", report["mining_records"])["value"]
        native_air = until(lambda: matched(rcon.command("execute if block 0 65 3 minecraft:air"), "Test passed"))
        after_mining = rcon.command("data get entity UnifiedProbe Pos")
        if after_mining != mining_position:
            raise RuntimeError("stationary mining changed native position")
        report["native_results"]["survival_mining"] = {
            "native_target": native_air, "position_before": mining_position,
            "position_after": after_mining, "stage": completed["stage"],
            "continuation_validated": completed["continuation_validated"],
        }
        trace.expect_disconnect()
        stage(probe, messages, "mining_disconnect", report["mining_records"])
        probe.wait(timeout=10)
        if probe.returncode != 0:
            raise RuntimeError("mining probe failed after disconnect")
        until(lambda: matched(rcon.command("execute unless entity @a[name=UnifiedProbe]"), "Test passed"))
        report["placement_fixture"] = {
            command: rcon.command(command) for command in ["setblock 2 65 0 minecraft:stone", "setblock 1 65 0 minecraft:air"]
        }
        for position, block in [("2 65 0", "stone"), ("1 65 0", "air")]:
            until(lambda: matched(rcon.command(f"execute if block {position} minecraft:{block}"), "Test passed"))
        report["placement_records"] = []
        probe = subprocess.Popen([str(REPO / "target/debug/examples/common_native_probe")], cwd=REPO, env=dict(env, VOXRIG_NATIVE_SCENARIO="placement"), stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=stderr_log, text=True, bufsize=1)
        messages = queue.Queue()
        thread = threading.Thread(target=pump, args=(probe.stdout, messages, probe_log), daemon=True)
        thread.start()
        stage(probe, messages, "placement_ready", report["placement_records"])
        report["placement_setup"] = {command: rcon.command(command) for command in ["gamemode survival UnifiedProbe", "tp UnifiedProbe 0.5 65 0.5 0 0", "clear UnifiedProbe", "give UnifiedProbe minecraft:dirt 3"]}
        stage(probe, messages, "placement_baseline", report["placement_records"])
        placement_position = rcon.command("data get entity UnifiedProbe Pos")
        stage(probe, messages, "placement_start", report["placement_records"])
        stage(probe, messages, "placement_observed", report["placement_records"])
        native_placed = until(lambda: matched(rcon.command("execute if block 1 65 0 minecraft:dirt"), "Test passed"))
        def material_after():
            response = rcon.command("data get entity UnifiedProbe Inventory")
            return response if 'id: "minecraft:dirt"' in response and "Slot: 0b" in response and re.search(r"(?:Count|count): 2(?:b)?(?:,|\s|})", response) else None
        native_material = until(material_after)
        after_placement = rcon.command("data get entity UnifiedProbe Pos")
        if after_placement != placement_position:
            raise RuntimeError("stationary placement changed native position")
        report["native_results"]["survival_placement"] = {"native_target": native_placed, "native_material": native_material, "position_before": placement_position, "position_after": after_placement}
        trace.expect_disconnect()
        stage(probe, messages, "placement_disconnect", report["placement_records"])
        probe.wait(timeout=10)
        if probe.returncode != 0:
            raise RuntimeError("placement probe failed after disconnect")
        until(lambda: matched(rcon.command("execute unless entity @a[name=UnifiedProbe]"), "Test passed"))
        report["inventory_records"] = []
        probe = subprocess.Popen([str(REPO / "target/debug/examples/common_native_probe")], cwd=REPO, env=dict(env, VOXRIG_NATIVE_SCENARIO="inventory"), stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=stderr_log, text=True, bufsize=1)
        messages = queue.Queue()
        thread = threading.Thread(target=pump, args=(probe.stdout, messages, probe_log), daemon=True)
        thread.start()
        stage(probe, messages, "swap_ready", report["inventory_records"])
        main_command = ("replaceitem entity UnifiedProbe inventory.0 minecraft:stone 3" if version == "1.16.1" else "item replace entity UnifiedProbe inventory.0 with minecraft:stone 3")
        hand_command = ("replaceitem entity UnifiedProbe hotbar.0 minecraft:dirt 2" if version == "1.16.1" else "item replace entity UnifiedProbe hotbar.0 with minecraft:dirt 2")
        report["inventory_setup"] = {command: rcon.command(command) for command in ["gamemode survival UnifiedProbe", "tp UnifiedProbe 0.5 65 0.5 0 0", "clear UnifiedProbe", main_command, hand_command]}
        def inventory_matches(expected):
            expected_inventory = dict(expected)
            expected_head = expected_inventory.pop(103, None) if version == "1.21.11" else None
            response = rcon.command("data get entity UnifiedProbe Inventory")
            stacks = outer_snbt_compounds(response)
            report["last_inventory_check"] = {"expected": expected, "inventory": response}
            if len(stacks) != len(expected_inventory):
                return None
            for slot, (item, count) in expected_inventory.items():
                if not any(re.search(rf"Slot: {slot}b(?:,|\s|}})", stack) and f'id: "{item}"' in stack and re.search(rf"(?:Count|count): {count}(?:b)?(?:,|\s|}})", stack) for stack in stacks):
                    return None
            if version == "1.21.11":
                # Original saved modern player data separates head equipment
                # from Inventory; it has no legacy Inventory Slot 103 entry.
                if expected_head is None:
                    head = rcon.command("execute unless data entity UnifiedProbe equipment.head")
                    if "Test passed" not in head:
                        report["last_inventory_check"]["equipment_head"] = head
                        return None
                else:
                    item, count = expected_head
                    head = rcon.command("data get entity UnifiedProbe equipment.head")
                    if len(outer_snbt_compounds(head)) != 1 or f'id: "{item}"' not in head or not re.search(rf"count: {count}(?:,|\s|}})", head):
                        report["last_inventory_check"]["equipment_head"] = head
                        return None
                return {"inventory": response, "equipment_head": head}
            return response
        report["inventory_fixture_native"] = until(lambda: inventory_matches({9: ("minecraft:stone", 3), 0: ("minecraft:dirt", 2)}))
        stage(probe, messages, "swap_baseline", report["inventory_records"])
        swap_position = rcon.command("data get entity UnifiedProbe Pos")
        stage(probe, messages, "swap_start", report["inventory_records"])
        stage(probe, messages, "swap_observed", report["inventory_records"])
        first_inventory = until(lambda: inventory_matches({9: ("minecraft:dirt", 2), 0: ("minecraft:stone", 3)}))
        report["inventory_creative_mode"] = rcon.command("gamemode creative UnifiedProbe")
        stage(probe, messages, "swap_creative", report["inventory_records"])
        stage(probe, messages, "swap_empty_observed", report["inventory_records"])
        second_inventory = until(lambda: inventory_matches({1: ("minecraft:dirt", 2), 0: ("minecraft:stone", 3)}))
        after_swap = rcon.command("data get entity UnifiedProbe Pos")
        if after_swap != swap_position:
            raise RuntimeError("ordinary inventory exchanges changed native position")
        report["native_results"]["inventory_swaps"] = {"survival_occupied_swap": first_inventory, "creative_empty_swap": second_inventory, "position_before": swap_position, "position_after": after_swap}
        trace.expect_disconnect()
        stage(probe, messages, "swap_disconnect", report["inventory_records"])
        probe.wait(timeout=10)
        if probe.returncode != 0:
            raise RuntimeError("inventory probe failed after disconnect")
        until(lambda: matched(rcon.command("execute unless entity @a[name=UnifiedProbe]"), "Test passed"))
        report["container_fixture"] = rcon.command("setblock 0 65 2 minecraft:chest")
        until(lambda: matched(rcon.command("execute if block 0 65 2 minecraft:chest"), "Test passed"))
        container_command = ("replaceitem block 0 65 2 container.0 minecraft:stone 3" if version == "1.16.1" else "item replace block 0 65 2 container.0 with minecraft:stone 3")
        report["container_fixture_items"] = rcon.command(container_command)
        def chest_matches(count):
            response = rcon.command("data get block 0 65 2 Items")
            return response if 'id: "minecraft:stone"' in response and "Slot: 0b" in response and re.search(rf"(?:Count|count): {count}(?:b)?(?:,|\s|}})", response) else None
        report["container_fixture_native"] = until(lambda: chest_matches(3))
        report["container_records"] = []
        probe = subprocess.Popen([str(REPO / "target/debug/examples/common_native_probe")], cwd=REPO, env=dict(env, VOXRIG_NATIVE_SCENARIO="container"), stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=stderr_log, text=True, bufsize=1)
        messages = queue.Queue()
        thread = threading.Thread(target=pump, args=(probe.stdout, messages, probe_log), daemon=True)
        thread.start()
        stage(probe,messages,"container_ready",report["container_records"])
        player_command = ("replaceitem entity UnifiedProbe inventory.0 minecraft:dirt 2" if version=="1.16.1" else "item replace entity UnifiedProbe inventory.0 with minecraft:dirt 2")
        report["container_setup"] = {command:rcon.command(command) for command in ["gamemode creative UnifiedProbe","tp UnifiedProbe 0.5 65 0.5 0 35","clear UnifiedProbe",player_command]}
        stage(probe,messages,"container_baseline",report["container_records"])
        container_position = rcon.command("data get entity UnifiedProbe Pos")
        container_rotation = rcon.command("data get entity UnifiedProbe Rotation")
        stage(probe,messages,"container_target_creative",report["container_records"])
        if rcon.command("data get entity UnifiedProbe Pos") != container_position or rcon.command("data get entity UnifiedProbe Rotation") != container_rotation:
            raise RuntimeError("read-only creative storage targeting changed native pose")
        stage(probe,messages,"container_open",report["container_records"])
        stage(probe,messages,"container_observed",report["container_records"])
        native_open = until(lambda: chest_matches(3))
        change_command = ("replaceitem block 0 65 2 container.0 minecraft:stone 7" if version=="1.16.1" else "item replace block 0 65 2 container.0 with minecraft:stone 7")
        report["container_change"] = rcon.command(change_command)
        native_changed = until(lambda: chest_matches(7))
        stage(probe,messages,"container_changed",report["container_records"])
        report["container_survival_mode"] = rcon.command("gamemode survival UnifiedProbe")
        stage(probe,messages,"container_target_survival",report["container_records"])
        if rcon.command("data get entity UnifiedProbe Pos") != container_position or rcon.command("data get entity UnifiedProbe Rotation") != container_rotation:
            raise RuntimeError("read-only survival storage targeting changed native pose")
        report["native_results"]["storage_targeting"] = {"target":[0,65,2],"fixture_state":"minecraft:chest[facing=north,type=single,waterlogged=false]","position":container_position,"rotation":container_rotation,"authority_limits":"RCON verifies unchanged native pose and setup fixture; actual targeting face/point is a model checked separately against original native clip methods, not a server hit acknowledgement or linked open result."}
        stage(probe,messages,"container_swap_survival",report["container_records"])
        stage(probe,messages,"container_swap_taken",report["container_records"])
        native_empty = until(lambda: matched(rcon.command("data get block 0 65 2 Items"),r"\[\]"))
        native_taken = until(lambda: inventory_matches({9:("minecraft:dirt",2),0:("minecraft:stone",7)}))
        report["container_creative_mode"] = rcon.command("gamemode creative UnifiedProbe")
        stage(probe,messages,"container_swap_creative",report["container_records"])
        stage(probe,messages,"container_swap_returned",report["container_records"])
        native_returned = until(lambda: chest_matches(7))
        native_return_inventory = until(lambda: inventory_matches({9:("minecraft:dirt",2)}))
        container_after = rcon.command("data get entity UnifiedProbe Pos")
        if container_after != container_position:
            raise RuntimeError("container observation changed native position")
        report["native_results"]["container_observation"] = {"opened_contents":native_open,"changed_contents":native_changed,"position_before":container_position,"position_after":container_after}
        report["native_results"]["container_swaps"] = {"survival_container":native_empty,"survival_inventory":native_taken,"creative_container":native_returned,"creative_inventory":native_return_inventory,"position_before":container_position,"position_after":container_after}
        report["container_pickup_survival_mode"] = rcon.command("gamemode survival UnifiedProbe")
        stage(probe,messages,"container_pickup_survival",report["container_records"])
        stage(probe,messages,"container_pickup_split",report["container_records"])
        pickup_split = until(lambda: chest_matches(3))
        report["container_pickup_creative_mode"] = rcon.command("gamemode creative UnifiedProbe")
        stage(probe,messages,"container_pickup_creative_one",report["container_records"])
        stage(probe,messages,"container_pickup_one",report["container_records"])
        pickup_one = until(lambda: chest_matches(4))
        stage(probe,messages,"container_pickup_creative_return",report["container_records"])
        stage(probe,messages,"container_pickup_returned",report["container_records"])
        pickup_return = until(lambda: chest_matches(7))
        report["native_results"]["container_pickups"] = {"survival_split_contents":pickup_split,"creative_one_contents":pickup_one,"creative_return_contents":pickup_return,"position_before":container_position,"position_after":rcon.command("data get entity UnifiedProbe Pos"),"authority_limits":"Original network PICKUP in both modes; independent RCON verifies storage counts. Fresh source/cursor and legacy comparison reply come from actual received packets; RCON does not verify cursor/menu ownership."}
        if report["native_results"]["container_pickups"]["position_after"] != container_position:
            raise RuntimeError("ordinary container pickup changed native position")
        report["container_transfer_survival_mode"] = rcon.command("gamemode survival UnifiedProbe")
        stage(probe,messages,"container_transfer_survival",report["container_records"])
        stage(probe,messages,"container_transfer_taken",report["container_records"])
        transfer_taken = until(lambda: inventory_matches({9:("minecraft:dirt",2),8:("minecraft:stone",7)}))
        transfer_empty = until(lambda: matched(rcon.command("data get block 0 65 2 Items"),r"\[\]"))
        report["container_transfer_creative_mode"] = rcon.command("gamemode creative UnifiedProbe")
        stage(probe,messages,"container_transfer_creative",report["container_records"])
        stage(probe,messages,"container_transfer_returned",report["container_records"])
        report["native_results"]["container_transfers"] = {"survival_inventory":transfer_taken,"survival_storage":transfer_empty,"creative_storage":until(lambda:chest_matches(7)),"creative_inventory":until(lambda:inventory_matches({9:("minecraft:dirt",2)})),"position_before":container_position,"position_after":rcon.command("data get entity UnifiedProbe Pos"),"authority_limits":"One original QUICK_MOVE per intent. RCON independently verifies exact counts and native reverse player order; fresh changed-slot receipts and unchanged actual cursor inspection are separate Client evidence, with matching actual legacy reply. No menu ownership or cursor assertion from RCON."}
        if report["native_results"]["container_transfers"]["position_after"] != container_position:
            raise RuntimeError("container shift transfer changed position")
        stage(probe,messages,"container_close_creative",report["container_records"])
        closed_change = ("replaceitem block 0 65 2 container.0 minecraft:stone 11" if version=="1.16.1" else "item replace block 0 65 2 container.0 with minecraft:stone 11")
        report["container_closed_change"] = rcon.command(closed_change)
        native_closed_contents = until(lambda: chest_matches(11))
        stage(probe,messages,"container_closed_change",report["container_records"])
        report["container_reopen_survival_mode"] = rcon.command("gamemode survival UnifiedProbe")
        stage(probe,messages,"container_reopen",report["container_records"])
        stage(probe,messages,"container_reopened",report["container_records"])
        report["container_close_survival_mode"] = report["container_reopen_survival_mode"]
        stage(probe,messages,"container_close_survival",report["container_records"])
        close_position = rcon.command("data get entity UnifiedProbe Pos")
        if close_position != container_position:
            raise RuntimeError("container close/reopen changed native position")
        report["native_results"]["container_open"] = {"target":[0,65,2],"creative_contents":native_open,"survival_contents":native_closed_contents,"position_before":container_position,"position_after":close_position,"authority_limits":"Same common empty-hand API in both modes: complete activation frame, actual fresh matching OPEN/full/cursor, modern actual processing ACK, distinct reopening and retained history. OPEN has no target coordinates: received screen facts do not prove causal target ownership. RCON independently verifies actual storage contents and unchanged native position."}
        report["native_results"]["container_close"] = {"closed_changed_contents":native_closed_contents,"player_inventory":until(lambda: inventory_matches({9:("minecraft:dirt",2)})),"position_before":container_position,"position_after":close_position,"authority_limits":"RCON verifies contents/inventory/position; same consumer verifies one complete close dispatch, no invented acknowledgement, closed-opening click refusal, and fresh distinct received reopening. No RCON menu-state assertion."}
        stage(probe,messages,"container_player_swap_survival",report["container_records"])
        stage(probe,messages,"container_player_taken",report["container_records"])
        player_taken=until(lambda:inventory_matches({0:("minecraft:dirt",2)}))
        report["container_player_creative_mode"]=rcon.command("gamemode creative UnifiedProbe")
        stage(probe,messages,"container_player_swap_creative",report["container_records"])
        stage(probe,messages,"container_player_returned",report["container_records"])
        player_returned=until(lambda:inventory_matches({9:("minecraft:dirt",2)}))
        player_position=rcon.command("data get entity UnifiedProbe Pos")
        if player_position != container_position:
            raise RuntimeError("player inventory resume after close changed native position")
        report["native_results"]["player_screen_after_close"]={"survival_inventory":player_taken,"creative_inventory":player_returned,"container_contents":until(lambda:chest_matches(11)),"position_before":container_position,"position_after":player_position,"authority_limits":"Native default player inventory exchanges after complete no-echo close. Received slots and explicit submitted-close UI basis, not a fabricated received active window/cursor/revision; RCON verifies exact contents/inventory/position."}
        report["player_pickup_survival_mode"] = rcon.command("gamemode survival UnifiedProbe")
        player_pickups = {}
        for submit, observed, inventory in [
            ("player_pickup_survival","player_pickup_taken",{}),
            ("player_pickup_creative_one","player_pickup_one",{0:("minecraft:dirt",1)}),
            ("player_pickup_creative_return","player_pickup_returned",{9:("minecraft:dirt",1),0:("minecraft:dirt",1)}),
            ("player_pickup_creative_retake","player_pickup_retaken",{9:("minecraft:dirt",1)}),
            ("player_pickup_creative_restore","player_pickup_restored",{9:("minecraft:dirt",2)}),
        ]:
            if submit == "player_pickup_creative_one":
                report["player_pickup_creative_mode"] = rcon.command("gamemode creative UnifiedProbe")
            stage(probe,messages,submit,report["container_records"])
            stage(probe,messages,observed,report["container_records"])
            player_pickups[observed] = until(lambda: inventory_matches(inventory))
        player_pickups["position_before"] = container_position
        player_pickups["position_after"] = rcon.command("data get entity UnifiedProbe Pos")
        player_pickups["authority_limits"] = "Actual ordinary player main/hotbar PICKUP after no-echo close, both modes and nonempty cursor. RCON independently verifies exact player inventory; cursor completion is separate actual receive evidence. No predicted receipt, invented window/revision/close acknowledgement or replay."
        if player_pickups["position_after"] != container_position:
            raise RuntimeError("ordinary player pickup changed native position")
        report["native_results"]["player_pickups"] = player_pickups
        report["player_transfer_survival_mode"] = rcon.command("gamemode survival UnifiedProbe")
        stage(probe,messages,"player_transfer_survival",report["container_records"])
        stage(probe,messages,"player_transfer_taken",report["container_records"])
        player_transfer_taken=until(lambda:inventory_matches({0:("minecraft:dirt",2)}))
        report["player_transfer_creative_mode"] = rcon.command("gamemode creative UnifiedProbe")
        stage(probe,messages,"player_transfer_creative",report["container_records"])
        stage(probe,messages,"player_transfer_returned",report["container_records"])
        report["native_results"]["player_transfers"]={"survival_inventory":player_transfer_taken,"creative_inventory":until(lambda:inventory_matches({9:("minecraft:dirt",2)})),"position_before":container_position,"position_after":rcon.command("data get entity UnifiedProbe Pos")}
        equipment_transfers={}
        for kind,item,count in [("pumpkin","minecraft:carved_pumpkin",7),("helmet","minecraft:diamond_helmet",1)]:
            command=(f"replaceitem entity UnifiedProbe inventory.1 {item} {count}" if version=="1.16.1" else f"item replace entity UnifiedProbe inventory.1 with {item} {count}")
            report[kind+"_transfer_fixture"]=rcon.command(command)
            before=until(lambda:inventory_matches({9:("minecraft:dirt",2),10:(item,count)}))
            report[kind+"_transfer_survival_mode"]=rcon.command("gamemode survival UnifiedProbe")
            stage(probe,messages,kind+"_transfer_survival",report["container_records"])
            stage(probe,messages,kind+"_transfer_equipped",report["container_records"])
            equipped=until(lambda:inventory_matches({9:("minecraft:dirt",2),103:(item,1),**({0:(item,6)} if kind=="pumpkin" else {})}))
            report[kind+"_transfer_creative_mode"]=rcon.command("gamemode creative UnifiedProbe")
            submit=kind+"_transfer_creative"+("_armor" if kind=="pumpkin" else "")
            observed=kind+"_transfer_"+("armor_returned" if kind=="pumpkin" else "returned")
            stage(probe,messages,submit,report["container_records"])
            stage(probe,messages,observed,report["container_records"])
            returned=until(lambda:inventory_matches({9:("minecraft:dirt",2),**({0:(item,7)} if kind=="pumpkin" else {10:(item,1)})}))
            if kind=="pumpkin":
                stage(probe,messages,"pumpkin_transfer_creative_hotbar",report["container_records"])
                stage(probe,messages,"pumpkin_transfer_hotbar_equipped",report["container_records"])
                returned=until(lambda:inventory_matches({9:("minecraft:dirt",2),103:(item,1),10:(item,6)}))
            # RCON and play packets have different server queues. Give the
            # already-written legacy comparison confirmation real native ticks
            # before an external fixture edit; this is not a gameplay ACK.
            def native_tick():
                raw = rcon.command("time query gametime")
                value = re.search(r"(\d+)\D*$", raw)
                if value is None:
                    raise RuntimeError("native tick query unavailable: " + raw)
                return int(value.group(1))
            tick_before = native_tick()
            tick_after = until(lambda: (tick if (tick := native_tick()) - tick_before >= 2 else None))
            cleared=rcon.command("clear UnifiedProbe "+item)
            report.setdefault("equipment_transfer_cleanup", {})[kind] = {"native_tick_before": tick_before, "native_tick_after": tick_after, "clear_response": cleared, "authority_limits": "Observed native fixture pacing between independent RCON and play queues; no processing or gameplay ACK is inferred."}
            until(lambda:inventory_matches({9:("minecraft:dirt",2)}))
            stage(probe,messages,"transfer_fixture_cleared",report["container_records"])
            equipment_transfers[kind]={"before":before,"survival_equipped":equipped,"creative_returned":returned,"clear_fixture":cleared}
        report["native_results"]["equipment_transfers"]={"items":equipment_transfers,"position_before":container_position,"position_after":rcon.command("data get entity UnifiedProbe Pos"),"authority_limits":"Original default equipment QUICK_MOVE on both modes. A single pumpkin-7 click equips one and transfers six; creative armor return first merges into existing hotbar stack; a following hotbar QUICK_MOVE equips one again and moves six to main. Default legacy Damage=0 NBT is preserved on diamond helmet. RCON checks exact real inventory counts separately from actual changed-slot receipts."}
        if any(report["native_results"][key]["position_after"]!=container_position for key in ("player_transfers","equipment_transfers")):
            raise RuntimeError("player/equipment shift transfer changed position")
        report["barrel_fixture"]=rcon.command("setblock 0 65 2 minecraft:barrel[facing=north,open=false]")
        report["barrel_fixture_items"]=rcon.command("replaceitem block 0 65 2 container.0 minecraft:stone 5" if version=="1.16.1" else "item replace block 0 65 2 container.0 with minecraft:stone 5")
        barrel_results={}
        for mode in ("creative","survival"):
            report["barrel_mode_"+mode]=rcon.command("gamemode "+mode+" UnifiedProbe")
            stage(probe,messages,"barrel_open_"+mode,report["container_records"])
            stage(probe,messages,"barrel_observed_"+mode,report["container_records"])
            opened=until(lambda:matched(rcon.command("execute if block 0 65 2 minecraft:barrel[open=true] run data get block 0 65 2 Items"),r'(?s)(?=.*minecraft:stone)(?=.*(?:Count|count): 5(?:b)?(?:,|\s|})).*'))
            stage(probe,messages,"barrel_close_"+mode,report["container_records"])
            closed=until(lambda:matched(rcon.command("execute if block 0 65 2 minecraft:barrel[open=false] run data get block 0 65 2 Items"),r'(?s)(?=.*minecraft:stone)(?=.*(?:Count|count): 5(?:b)?(?:,|\s|})).*'))
            barrel_results[mode]={"opened":opened,"closed":closed}
        barrel_position=rcon.command("data get entity UnifiedProbe Pos")
        if barrel_position!=container_position:raise RuntimeError("barrel activation changed native position")
        report["native_results"]["barrel_activation"]={"modes":barrel_results,"player_inventory":until(lambda:inventory_matches({9:("minecraft:dirt",2)})),"position_before":container_position,"position_after":barrel_position,"authority_limits":"Native RCON verifies real barrel open/closed boolean and unchanged stone 5 contents/player inventory/pose. Same Client independently receives matching menus/full/cursor/modern processing and actual open flag cache. Open-property transitions are outline/menu-compatible, not a target-linked acknowledgement or menu ownership proof."}
        # The existing read-only command emits the same public PlayerObservation
        # on both adapters. Only external fixture syntax is version-specific.
        item_data_results = {}
        report["native_results"]["item_data_observation"] = item_data_results
        for mode, count, marker in [("survival", 7, 17), ("creative", 8, 18)]:
            baseline = stage(probe, messages, "transfer_fixture_cleared", report["container_records"])["value"]
            observed_name = "VoxrigData" + mode.capitalize()
            result = {"baseline": baseline, "fixture": {}, "name": observed_name, "marker": marker, "count": count}
            item_data_results[mode] = result
            boundary = trace.mark()
            result["position_before"] = rcon.command("data get entity UnifiedProbe Pos")
            result["rotation_before"] = rcon.command("data get entity UnifiedProbe Rotation")
            result["fixture"]["mode"] = rcon.command("gamemode " + mode + " UnifiedProbe")
            result["fixture"]["clear"] = rcon.command("clear UnifiedProbe")
            command = (f'replaceitem entity UnifiedProbe inventory.0 minecraft:stone{{VoxrigProbe:{marker},Damage:7,display:{{Name:\'{{"text":"{observed_name}"}}\'}}}} {count}'
                       if version == "1.16.1" else
                       f'item replace entity UnifiedProbe inventory.0 with minecraft:stone[max_stack_size=16,custom_data={{VoxrigProbe:{marker}}},custom_name={{text:"{observed_name}"}},bundle_contents=[{{id:"minecraft:stone",count:2,components:{{"minecraft:custom_name":"NestedProbe","minecraft:custom_data":{{VoxrigNestedProbe:19}}}}}}],enchantments={{"minecraft:unbreaking":2}},written_book_content={{title:"VoxrigBook",author:"Voxrig",pages:["ComplexProbe"],resolved:true}}] {count}')
            result["fixture"]["item"] = rcon.command(command)
            result["native_inventory"] = until(lambda: inventory_matches({9:("minecraft:stone",count)}))
            path = "tag" if version == "1.16.1" else 'components."minecraft:custom_data"'
            result["native_marker"] = until(lambda:matched(rcon.command(f'data get entity UnifiedProbe Inventory[{{Slot:9b}}].{path}.VoxrigProbe'),rf'\b{marker}\b'))
            name_path = "tag.display.Name" if version == "1.16.1" else 'components."minecraft:custom_name"'
            result["native_name"] = until(lambda:matched(rcon.command(f'data get entity UnifiedProbe Inventory[{{Slot:9b}}].{name_path}'),observed_name))
            property_path = "tag.Damage" if version == "1.16.1" else 'components."minecraft:max_stack_size"'
            expected_property_value = 7 if version == "1.16.1" else 16
            result["native_property_value"] = until(lambda:matched(rcon.command(f'data get entity UnifiedProbe Inventory[{{Slot:9b}}].{property_path}'),rf'\b{expected_property_value}\b'))
            if version == "1.21.11":
                component_path='Inventory[{Slot:9b}].components.'
                result["native_nested_marker"]=until(lambda:matched(rcon.command('data get entity UnifiedProbe '+component_path+'"minecraft:bundle_contents"[0].components."minecraft:custom_data".VoxrigNestedProbe'),r'\b19\b'))
                result["native_nested_count"]=until(lambda:matched(rcon.command('data get entity UnifiedProbe '+component_path+'"minecraft:bundle_contents"[0].count'),r'\b2\b'))
                result["native_enchantment"]=until(lambda:matched(rcon.command('data get entity UnifiedProbe '+component_path+'"minecraft:enchantments"."minecraft:unbreaking"'),r'\b2\b'))
                result["native_book"]=until(lambda:matched(rcon.command('data get entity UnifiedProbe '+component_path+'"minecraft:written_book_content"'), 'ComplexProbe'))
            marker_key = b"VoxrigProbe"
            encoded_marker = bytes([3])+len(marker_key).to_bytes(2,"big")+marker_key+marker.to_bytes(4,"big",signed=True)
            def received_item_data():
                player = stage(probe, messages, "transfer_fixture_cleared", report["container_records"])["value"]
                slot = player["inventory"]["slots"][9]
                if player["game_mode"] != mode or slot is None or slot["source"]["kind"] != "received" or slot["source"]["sequence"] <= baseline["receive_sequence"]:
                    return None
                value = slot["value"]
                if value["kind"] != "item" or value["item"]["name"] != "minecraft:stone" or value["item"]["count"] != count:
                    return None
                data = value["item"]["data"]
                if version == "1.16.1":
                    if data["kind"] != "legacy_nbt": return None
                    encoded = bytes(data["bytes"])
                    if encoded_marker not in encoded or observed_name.encode() not in encoded: return None
                else:
                    if data["kind"] != "modern_components": return None
                    patch = data["patch"]
                    added = {c["definition"]["name"]:c for c in patch["added"]}
                    if set(added) != {"minecraft:max_stack_size", "minecraft:custom_data", "minecraft:custom_name", "minecraft:bundle_contents", "minecraft:enchantments", "minecraft:written_book_content"} or patch["removed"]: return None
                    if added["minecraft:max_stack_size"]["bytes"] != [16]: return None
                    if encoded_marker not in bytes(added["minecraft:custom_data"]["bytes"]) or observed_name.encode() not in bytes(added["minecraft:custom_name"]["bytes"]): return None
                    nested=bytes(added["minecraft:bundle_contents"]["bytes"])
                    nested_key=b"VoxrigNestedProbe"
                    encoded_nested=bytes([3])+len(nested_key).to_bytes(2,"big")+nested_key+(19).to_bytes(4,"big")
                    if b"NestedProbe" not in nested or encoded_nested not in nested: return None
                    if b"ComplexProbe" not in bytes(added["minecraft:written_book_content"]["bytes"]): return None
                    if not added["minecraft:enchantments"]["bytes"]: return None
                return player
            result["received"] = until(received_item_data)
            registry_snapshot = stage(probe, messages, "registry_state", report["container_records"])["value"]
            result["server_registry"] = verify_received_registries(registry_snapshot, result["received"], trace, version)
            custom=registry_snapshot["custom_data"]
            marker_value=next((entry["value"] for entry in custom["root"]["value"]["entries"] if entry["key"]=="VoxrigProbe"),None)
            custom_source=registry_snapshot["custom_data_source"]
            if marker_value!={"type":"int","value":marker} or custom_source["kind"]!="received" or custom_source["sequence"]<result["received"]["inventory"]["slots"][9]["source"]["sequence"] or registry_snapshot["custom_data_session"]!=result["received"]["session"] or registry_snapshot["custom_data_item"]!=result["received"]["inventory"]["slots"][9]["value"]:
                raise RuntimeError("decoded common custom-data marker/session/source differs from fresh actual item")
            if (custom["persistent_crc32c"] is None)!=(version=="1.16.1"):
                raise RuntimeError("custom-data value hash invented legacy availability or lost modern value")
            result["decoded_custom_data"]={"data":custom,"source":custom_source,"session":registry_snapshot["custom_data_session"],"item":registry_snapshot["custom_data_item"],"native_marker":result["native_marker"],"authority_limits":"Same common item.custom_data accessor returns typed marker from actual received item in both modes/versions; original RCON confirms marker. Actual current item/source is retained; a newer unchanged receipt is not relabeled as the earlier ordinal. Pure modern NBT persistent CRC32C is separately oracle-verified, not an inventory hash/permission or general item interpretation."}
            properties = registry_snapshot["item_properties"]
            expected_properties = {"max_stack_size":64 if version=="1.16.1" else 16,"max_damage":0,"damage":7 if version=="1.16.1" else 0,"damageable":False,"damaged":False,"stackable":True}
            if properties != expected_properties: raise RuntimeError("common effective item properties disagree with native fixture")
            result["decoded_item_properties"]={"value":properties,"source":custom_source,"session":registry_snapshot["custom_data_session"],"item":registry_snapshot["custom_data_item"],"native_property_value":result["native_property_value"],"authority_limits":"Same public item.properties getter, actual received data/source preserved. Independent RCON confirms legacy Damage=7 or modern max_stack_size=16. Original prototype/scalar/removal oracle separately verifies native getters; not item equality, inventory/cache hash, slot rules or nondefault action permission."}
            result["position_after"] = rcon.command("data get entity UnifiedProbe Pos")
            result["rotation_after"] = rcon.command("data get entity UnifiedProbe Rotation")
            if result["position_before"] != result["position_after"] or result["rotation_before"] != result["rotation_after"]:
                raise RuntimeError("item-data observation changed native pose")
            result["frames"] = [f for f in trace.since(boundary) if f["phase"] == "play"]
            mutation_ids = (0x09,0x0a,0x27) if version == "1.16.1" else (0x11,0x12,0x37)
            if any(f["direction"] == "serverbound" and f["packet_id"] in mutation_ids for f in result["frames"]):
                raise RuntimeError("read-only item-data observation wrote inventory/close frames")
            result["authority_limits"] = "Same public Client observation receives fresh exact item identity/count and original legacy NBT or modern custom-data/name, nested named item, registry-referencing enchantment and book patch. Independent RCON fields confirm values and unchanged pose. Read-only original frames contain no outgoing click/close/creative-slot mutation. Raw reference retention does not resolve arbitrary live registry bindings or authorize component-bearing gameplay."
            pickup_boundary = trace.mark()
            data_pickup = {"client":stage(probe, messages, "item_data_pickup_" + mode, report["container_records"])["value"]}
            result["data_pickup"] = data_pickup
            data_pickup["native_restored_inventory"] = until(lambda: inventory_matches({9:("minecraft:stone",count)}))
            data_pickup["native_restored_marker"] = until(lambda:matched(rcon.command(f'data get entity UnifiedProbe Inventory[{{Slot:9b}}].{path}.VoxrigProbe'),rf'\b{marker}\b'))
            data_pickup["native_restored_name"] = until(lambda:matched(rcon.command(f'data get entity UnifiedProbe Inventory[{{Slot:9b}}].{name_path}'),observed_name))
            data_pickup["frames"] = [f for f in trace.since(pickup_boundary) if f["phase"] == "play"]
            clicks = [f for f in data_pickup["frames"] if f["direction"] == "serverbound" and f["packet_id"] == (0x09 if version == "1.16.1" else 0x11)]
            if len(clicks) != 3 or not data_pickup["client"]["native_data_equivalent"]:
                raise RuntimeError("data PICKUP did not preserve fields across exactly3 explicit clicks")
            data_pickup["authority_limits"] = "Same public mode handle performs split/one-place/all-return. Each fresh source/cursor packet matches native data fields and explicit counts; original server inventory/name/marker confirm restored state. Modern deliberate revision mismatch requests full actual resync; empty comparison marker is not a cursor receipt or computed item hash."
            transfer_boundary = trace.mark()
            data_transfer = {"client":stage(probe, messages, "item_data_transfer_" + mode, report["container_records"])["value"]}
            result["data_transfer"] = data_transfer
            data_transfer["native_restored_inventory"] = until(lambda: inventory_matches({9:("minecraft:stone",count)}))
            data_transfer["native_restored_marker"] = until(lambda:matched(rcon.command(f'data get entity UnifiedProbe Inventory[{{Slot:9b}}].{path}.VoxrigProbe'),rf'\b{marker}\b'))
            data_transfer["native_restored_name"] = until(lambda:matched(rcon.command(f'data get entity UnifiedProbe Inventory[{{Slot:9b}}].{name_path}'),observed_name))
            data_transfer["frames"] = [f for f in trace.since(transfer_boundary) if f["phase"] == "play"]
            clicks = [f for f in data_transfer["frames"] if f["direction"] == "serverbound" and f["packet_id"] == (0x09 if version == "1.16.1" else 0x11)]
            if len(clicks) != 2 or not data_transfer["client"]["native_data_equivalent"]:
                raise RuntimeError("data QUICK_MOVE did not preserve fields across exactly2 explicit transfers")
            data_transfer["authority_limits"] = "Same public mode handle moves data-bearing stack to hotbar and returns it. Fresh changed source/destination packets match independent counts and native data fields; original RCON confirms restored inventory/name/marker. Cursor remains actually empty; QUICK_MOVE does not require a synthesized fresh cursor receipt. Modified equippable routing and nondefault equipped-item pickup rules remain pending."
            swap_boundary = trace.mark()
            data_swap = {"client": stage(probe, messages, "item_data_swap_" + mode, report["container_records"])["value"]}
            result["data_swap"] = data_swap
            data_swap["native_inventory"] = until(lambda: inventory_matches({0:("minecraft:stone",count)}))
            data_swap["native_marker"] = until(lambda:matched(rcon.command(f'data get entity UnifiedProbe Inventory[{{Slot:0b}}].{path}.VoxrigProbe'),rf'\b{marker}\b'))
            data_swap["native_name"] = until(lambda:matched(rcon.command(f'data get entity UnifiedProbe Inventory[{{Slot:0b}}].{name_path}'),observed_name))
            data_swap["native_property_value"] = until(lambda:matched(rcon.command(f'data get entity UnifiedProbe Inventory[{{Slot:0b}}].{property_path}'),rf'\b{expected_property_value}\b'))
            if version == "1.21.11":
                component_path = 'Inventory[{Slot:0b}].components.'
                data_swap["native_nested_marker"] = until(lambda:matched(rcon.command('data get entity UnifiedProbe '+component_path+'"minecraft:bundle_contents"[0].components."minecraft:custom_data".VoxrigNestedProbe'),r'\b19\b'))
                data_swap["native_enchantment"] = until(lambda:matched(rcon.command('data get entity UnifiedProbe '+component_path+'"minecraft:enchantments"."minecraft:unbreaking"'),r'\b2\b'))
                data_swap["native_book"] = until(lambda:matched(rcon.command('data get entity UnifiedProbe '+component_path+'"minecraft:written_book_content"'), 'ComplexProbe'))
            data_swap["frames"] = [f for f in trace.since(swap_boundary) if f["phase"] == "play"]
            clicks = [f for f in data_swap["frames"] if f["direction"] == "serverbound" and f["packet_id"] == (0x09 if version == "1.16.1" else 0x11)]
            if len(clicks) != 1 or not data_swap["client"]["native_item_equivalent"]:
                raise RuntimeError("data swap did not complete exactly one click with native item equivalence")
            data_swap["authority_limits"] = "Same public mode handle exchanges received data-bearing player stack once. Fresh destination receipts and owning registries establish native semantic item equivalence; independent original-server RCON confirms destination/count/name/custom marker/property and modern nested/enchantment/book data. No predicted hashes or cache receipts establish outcome."
            # Return through a second explicit SWAP and actual resync, rather than
            # inferring a hotbar clear from RCON: legacy /clear did not transmit a
            # fresh screen36 receipt after this data-bearing exchange.
            return_boundary = trace.mark()
            data_swap["returned"] = stage(probe, messages, "item_data_return_" + mode, report["container_records"])["value"]
            data_swap["native_returned_inventory"] = until(lambda: inventory_matches({9:("minecraft:stone",count)}))
            data_swap["native_returned_marker"] = until(lambda:matched(rcon.command(f'data get entity UnifiedProbe Inventory[{{Slot:9b}}].{path}.VoxrigProbe'),rf'\b{marker}\b'))
            return_frames = [f for f in trace.since(return_boundary) if f["phase"] == "play"]
            return_clicks = [f for f in return_frames if f["direction"] == "serverbound" and f["packet_id"] == (0x09 if version == "1.16.1" else 0x11)]
            if len(return_clicks) != 1 or not data_swap["returned"]["native_item_equivalent"]:
                raise RuntimeError("data return did not complete exactly one explicit click")
            data_swap["return_frames"] = return_frames

            if version == "1.21.11":
                equipment = {"fixture": {}}
                result["modified_equipment_transfer"] = equipment
                equipment["fixture"]["clear"] = rcon.command("clear UnifiedProbe")
                equipment_baseline = stage(probe, messages, "transfer_fixture_cleared", report["container_records"])["value"]
                equipment["fixture"]["item"] = rcon.command('item replace entity UnifiedProbe inventory.0 with minecraft:stone[equippable={slot:"head",allowed_entities:["minecraft:player"]},custom_data={VoxrigEquipmentProbe:41}] 3')
                equipment["native_before"] = until(lambda: inventory_matches({9:("minecraft:stone",3)}))
                def received_equipment():
                    player = stage(probe, messages, "transfer_fixture_cleared", report["container_records"])["value"]
                    slot = player["inventory"]["slots"][9]
                    if player["game_mode"] != mode or slot is None or slot["source"]["kind"] != "received" or slot["source"]["sequence"] <= equipment_baseline["receive_sequence"]:
                        return None
                    value = slot["value"]
                    if value["kind"] != "item" or value["item"]["count"] != 3 or value["item"]["name"] != "minecraft:stone":
                        return None
                    data = value["item"]["data"]
                    if data["kind"] != "modern_components" or "minecraft:equippable" not in {c["definition"]["name"] for c in data["patch"]["added"]}:
                        return None
                    return player
                equipment["received_before"] = until(received_equipment)
                equipment_boundary = trace.mark()
                equipment["client"] = stage(probe, messages, "item_equipment_transfer_" + mode, report["container_records"])["value"]
                equipment["native_after"] = until(lambda: inventory_matches({103:("minecraft:stone",1),0:("minecraft:stone",2)}))
                equipment["native_head_marker"] = until(lambda: matched(rcon.command('data get entity UnifiedProbe equipment.head.components."minecraft:custom_data".VoxrigEquipmentProbe'), r'\b41\b'))
                equipment["native_hotbar_marker"] = until(lambda: matched(rcon.command('data get entity UnifiedProbe Inventory[{Slot:0b}].components."minecraft:custom_data".VoxrigEquipmentProbe'), r'\b41\b'))
                equipment["native_head_equippable"] = until(lambda: matched(rcon.command('data get entity UnifiedProbe equipment.head.components."minecraft:equippable"'), r'head'))
                equipment["frames"] = [f for f in trace.since(equipment_boundary) if f["phase"] == "play"]
                clicks = [f for f in equipment["frames"] if f["direction"] == "serverbound" and f["packet_id"] == 0x11]
                if len(clicks) != 1 or not equipment["client"]["native_data_equivalent"]:
                    raise RuntimeError("modified equipment did not complete one transfer preserving actual data")
                equipment["authority_limits"] = "Effective equippable routes a received stone stack to head1/hotbar2 through the same public mode handle. Fresh changed-slot receipts and native fields/counts are checked; independent original RCON verifies head/hotbar data. This does not verify nondefault armor extraction or arbitrary item activation."


            armor = {"fixture": {}}
            result["armor_transfer"] = armor
            if version == "1.16.1":
                # Retain the earlier stack as a control in hotbar after moving
                # it through actual common transfer receipts. Original /clear
                # does not always emit fresh receipts after comparison resync.
                armor["fixture"]["evacuated"] = stage(probe, messages, "item_data_evacuate_" + mode, report["container_records"])["value"]
            else:
                armor["fixture"]["clear"] = rcon.command("clear UnifiedProbe")
            armor_control = {0:("minecraft:stone",count)} if version == "1.16.1" else {}
            armor_baseline = stage(probe, messages, "armor_fixture_state", report["container_records"])["value"]
            enchantment = "binding_curse" if mode == "creative" else "unbreaking"
            armor_marker = 992
            armor_command = (
                'replaceitem entity UnifiedProbe armor.head minecraft:diamond_helmet{Damage:7,VoxrigArmorProbe:992,Enchantments:[{id:"minecraft:' + enchantment + '",lvl:1s}]} 1'
                if version == "1.16.1" else
                'item replace entity UnifiedProbe armor.head with minecraft:diamond_helmet[damage=7,custom_data={VoxrigArmorProbe:992},enchantments={"minecraft:' + enchantment + '":1}] 1'
            )
            armor["fixture"]["item"] = rcon.command(armor_command)
            armor["native_before"] = until(lambda: inventory_matches({**armor_control,103:("minecraft:diamond_helmet",1)}))
            armor_key = b"VoxrigArmorProbe"
            armor_bytes = bytes([3])+len(armor_key).to_bytes(2,"big")+armor_key+armor_marker.to_bytes(4,"big",signed=True)
            def received_armor(expected_mode):
                player = stage(probe, messages, "armor_fixture_state", report["container_records"])["value"]
                slot = player["inventory"]["slots"][5]
                main = player["inventory"]["slots"][9]
                if player["game_mode"] != expected_mode or slot is None or slot["source"]["kind"] != "received" or slot["source"]["sequence"] <= armor_baseline["receive_sequence"] or main is None or main["value"]["kind"] != "empty":
                    return None
                value = slot["value"]
                if value["kind"] != "item" or value["item"]["name"] != "minecraft:diamond_helmet" or value["item"]["count"] != 1:
                    return None
                data = value["item"]["data"]
                raw = bytes(data["bytes"]) if version == "1.16.1" else next((bytes(c["bytes"]) for c in data["patch"]["added"] if c["definition"]["name"] == "minecraft:custom_data"), b"")
                return player if armor_bytes in raw else None
            armor["received_before"] = until(lambda: received_armor(mode))
            if mode == "creative":
                armor["survival_mode"] = rcon.command("gamemode survival UnifiedProbe")
                until(lambda: received_armor("survival"))
                refusal_boundary = trace.mark()
                armor["survival_refusal"] = stage(probe, messages, "item_armor_refuse", report["container_records"])["value"]
                armor["refusal_frames"] = [f for f in trace.since(refusal_boundary) if f["phase"] == "play"]
                if any(f["direction"] == "serverbound" and f["packet_id"] == (0x09 if version == "1.16.1" else 0x11) for f in armor["refusal_frames"]):
                    raise RuntimeError("survival binding armor refusal sent a click")
                armor["native_after_refusal"] = until(lambda: inventory_matches({**armor_control,103:("minecraft:diamond_helmet",1)}))
                armor["creative_mode"] = rcon.command("gamemode creative UnifiedProbe")
                until(lambda: received_armor("creative"))
            armor_boundary = trace.mark()
            armor["client"] = stage(probe, messages, "item_armor_transfer_" + mode, report["container_records"])["value"]
            armor["native_after"] = until(lambda: inventory_matches({**armor_control,9:("minecraft:diamond_helmet",1)}))
            armor_data_path = 'Inventory[{Slot:9b}].tag.VoxrigArmorProbe' if version == "1.16.1" else 'Inventory[{Slot:9b}].components."minecraft:custom_data".VoxrigArmorProbe'
            armor_damage_path = 'Inventory[{Slot:9b}].tag.Damage' if version == "1.16.1" else 'Inventory[{Slot:9b}].components."minecraft:damage"'
            armor["native_marker"] = until(lambda: matched(rcon.command('data get entity UnifiedProbe ' + armor_data_path), r'\b992\b'))
            armor["native_damage"] = until(lambda: matched(rcon.command('data get entity UnifiedProbe ' + armor_damage_path), r'\b7\b'))
            armor["frames"] = [f for f in trace.since(armor_boundary) if f["phase"] == "play"]
            if len([f for f in armor["frames"] if f["direction"] == "serverbound" and f["packet_id"] == (0x09 if version == "1.16.1" else 0x11)]) != 1 or not armor["client"]["native_data_equivalent"]:
                raise RuntimeError("armor extraction lacks one completed metadata-preserving transfer")
            armor["authority_limits"] = "Same common Client modes extract received damaged enchanted armor with fresh source/destination and independent native RCON counts/marker/damage. Survival binding refusal sends no click; creative bypass preserves that same item. Legacy completion additionally requires actual native comparison reply."


            held_transfer = {"fixture": {}}
            result["held_cursor_transfer"] = held_transfer
            held_boundary = trace.mark()
            held_transfer["take"] = stage(probe, messages, "held_cursor_take_" + mode, report["container_records"])["value"]
            held_transfer["native_holding_inventory"] = until(lambda: inventory_matches(armor_control))
            held_transfer["fixture"]["source"] = rcon.command("replaceitem entity UnifiedProbe inventory.0 minecraft:dirt 7" if version == "1.16.1" else "item replace entity UnifiedProbe inventory.0 with minecraft:dirt 7")
            held_transfer["native_before"] = until(lambda: inventory_matches({**armor_control,9:("minecraft:dirt",7)}))
            def received_held_source():
                player = stage(probe, messages, "armor_fixture_state", report["container_records"])["value"]
                source = player["inventory"]["slots"][9]
                cursor = player["inventory"]["cursor"]
                if player["game_mode"] != mode or source is None or source["source"]["kind"] != "received" or source["source"]["sequence"] <= held_transfer["take"]["record"]["send"]["after_sequence"]:
                    return None
                value = source["value"]
                if value["kind"] != "item" or value["item"]["name"] != "minecraft:dirt" or value["item"]["count"] != 7 or cursor is None or cursor["source"]["kind"] != "received" or cursor["value"]["kind"] != "item" or cursor["value"]["item"]["name"] != "minecraft:diamond_helmet" or cursor["value"]["item"]["count"] != 1:
                    return None
                return player
            held_transfer["received_before"] = until(received_held_source)
            held_transfer["client"] = stage(probe, messages, "held_cursor_transfer_" + mode, report["container_records"])["value"]
            held_destination = held_transfer["client"]["destination"] - 36
            held_native = {**armor_control,held_destination:("minecraft:dirt",7)}
            held_transfer["native_after"] = until(lambda: inventory_matches(held_native))
            held_transfer["restore"] = stage(probe, messages, "held_cursor_restore_" + mode, report["container_records"])["value"]
            held_transfer["native_restored"] = until(lambda: inventory_matches({**held_native,10:("minecraft:diamond_helmet",1)}))
            held_transfer["native_restored_marker"] = until(lambda: matched(rcon.command('data get entity UnifiedProbe ' + armor_data_path.replace('Slot:9b', 'Slot:10b')), r'\b992\b'))
            held_transfer["native_restored_damage"] = until(lambda: matched(rcon.command('data get entity UnifiedProbe ' + armor_damage_path.replace('Slot:9b', 'Slot:10b')), r'\b7\b'))
            held_transfer["frames"] = [f for f in trace.since(held_boundary) if f["phase"] == "play"]
            if len([f for f in held_transfer["frames"] if f["direction"] == "serverbound" and f["packet_id"] == (0x09 if version == "1.16.1" else 0x11)]) != 3 or not held_transfer["client"]["cursor_native_equivalent"]:
                raise RuntimeError("held cursor take/transfer/restore lacks three complete native clicks")
            held_transfer["authority_limits"] = "Same common Client takes damaged enchanted armor to received cursor, transfers a separate stack while preserving that cursor, then restores the carried item. Actual slot/cursor/reply receipts and independent native RCON inventory/counts/restored damage/marker are checked. RCON cannot directly read menu cursor; restored native item provides separate end-to-end metadata evidence. Legacy control stack is retained."

            crafting = {"fixture": {}}
            result["crafting_inputs"] = crafting
            crafting["fixture"]["material"] = rcon.command("replaceitem entity UnifiedProbe inventory.0 minecraft:oak_planks 3" if version == "1.16.1" else "item replace entity UnifiedProbe inventory.0 with minecraft:oak_planks 3")
            def received_crafting_material():
                player = stage(probe, messages, "armor_fixture_state", report["container_records"])["value"]
                slot = player["inventory"]["slots"][9]
                if player["game_mode"] != mode or slot is None or slot["source"]["kind"] != "received":
                    return None
                value = slot["value"]
                return player if value["kind"] == "item" and value["item"]["name"] == "minecraft:oak_planks" and value["item"]["count"] == 3 else None
            crafting["received_before"] = until(received_crafting_material)
            crafting_boundary = trace.mark()
            crafting["client"] = stage(probe, messages, "crafting_input_" + mode, report["container_records"])["value"]
            crafting["native_after"] = until(lambda: inventory_matches({**held_native,9:("minecraft:oak_planks",3),10:("minecraft:diamond_helmet",1)}))
            crafting["frames"] = [f for f in trace.since(crafting_boundary) if f["phase"] == "play"]
            if len([f for f in crafting["frames"] if f["direction"] == "serverbound" and f["packet_id"] == (0x09 if version == "1.16.1" else 0x11)]) != 5 or len(crafting["client"]["steps"]) != 5:
                raise RuntimeError("crafting input round trip lacks five native clicks")
            crafting["authority_limits"] = "Same public Client/mode handles place and retrieve one actual crafting ingredient, observe the native displayed result and its disappearance, and restore three planks. Fresh input/cursor/result receipts, actual native comparison replies and independent RCON final inventory are checked. No result take or recipe consumption is claimed."

            result["fixture"]["clear_after"] = rcon.command("clear UnifiedProbe")
            result["fixture"]["restore"] = rcon.command("replaceitem entity UnifiedProbe inventory.0 minecraft:dirt 2" if version == "1.16.1" else "item replace entity UnifiedProbe inventory.0 with minecraft:dirt 2")
        trace.expect_disconnect()
        stage(probe,messages,"container_disconnect",report["container_records"])
        probe.wait(timeout=10)
        if probe.returncode != 0:
            raise RuntimeError("container probe failed after disconnect")
        cursor_returns = {}
        report["native_results"]["cursor_return_close"] = cursor_returns
        return_items = [("minecraft:stone", 5, False), ("minecraft:diamond_helmet", 1, False), ("minecraft:stone", 5, True)]
        if version == "1.21.11":
            return_items.append(("minecraft:bundle", 1, False))
        for mode in ("survival", "creative"):
            for item, count, with_data in return_items:
                until(lambda:matched(rcon.command("execute unless entity @a[name=UnifiedProbe]"),"Test passed"))
                probe = subprocess.Popen([str(REPO / "target/debug/examples/common_native_probe")], cwd=REPO, env=dict(env, VOXRIG_NATIVE_SCENARIO="container"), stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=stderr_log, text=True, bufsize=1)
                messages = queue.Queue()
                thread = threading.Thread(target=pump,args=(probe.stdout,messages,probe_log),daemon=True)
                thread.start()
                stage(probe,messages,"container_ready",report["container_records"])
                key = mode + "/" + item + ("/data" if with_data else "")
                result = {"fixture": {}}
                cursor_returns[key] = result
                for command in ("clear UnifiedProbe", "kill @e[type=minecraft:item]", "gamemode " + mode + " UnifiedProbe", "tp UnifiedProbe 0.5 65 0.5 0 35"):
                    result["fixture"][command] = rcon.command(command)
                fixture_item = item
                maximum = 64
                if with_data:
                    if version == "1.16.1":
                        fixture_item += "{VoxrigCursorProbe:23,display:{Name:'\"CursorReturnData\"'}}"
                    else:
                        maximum = 16
                        fixture_item += '[minecraft:max_stack_size=16,minecraft:custom_data={VoxrigCursorProbe:23},minecraft:custom_name={text:"CursorReturnData"},minecraft:bundle_contents=[{id:"minecraft:stone",count:2,components:{"minecraft:custom_data":{CursorNested:31}}}]]'
                if item == "minecraft:stone":
                    command = (f"replaceitem entity UnifiedProbe inventory.0 {fixture_item} {maximum-1}" if version == "1.16.1" else f"item replace entity UnifiedProbe inventory.0 with {fixture_item} {maximum-1}")
                    result["fixture"][command] = rcon.command(command)
                command = (f"replaceitem block 0 65 2 container.0 {fixture_item} {count}" if version == "1.16.1" else f"item replace block 0 65 2 container.0 with {fixture_item} {count}")
                result["fixture"][command] = rcon.command(command)
                stage(probe,messages,"cursor_close_audit_open_" + mode,report["container_records"])
                result["opened"] = stage(probe,messages,"cursor_close_audit_opened",report["container_records"])["value"]
                stage(probe,messages,"cursor_close_audit_pickup",report["container_records"])
                result["held"] = stage(probe,messages,"cursor_return_holding",report["container_records"])["value"]
                result["position_before"] = rcon.command("data get entity UnifiedProbe Pos")
                result["rotation_before"] = rcon.command("data get entity UnifiedProbe Rotation")
                boundary = trace.mark()
                result["close"] = stage(probe,messages,"cursor_return_close",report["container_records"])["value"]
                result["inventory_after"] = until(lambda:inventory_matches({9:(item,maximum),10:(item,count-1)} if item == "minecraft:stone" else {9:(item,1)}))
                if not result["close"]["native_data_equivalent"]:
                    raise RuntimeError("cursor return did not preserve actual native data")
                if with_data:
                    metadata_path = "tag.VoxrigCursorProbe" if version == "1.16.1" else 'components."minecraft:custom_data".VoxrigCursorProbe'
                    name_path = "tag.display.Name" if version == "1.16.1" else 'components."minecraft:custom_name"'
                    result["native_metadata"] = {}
                    for native_slot in (9,10):
                        result["native_metadata"][str(native_slot)] = {
                            "marker":until(lambda:matched(rcon.command(f'data get entity UnifiedProbe Inventory[{{Slot:{native_slot}b}}].{metadata_path}'),r'\b23\b')),
                            "name":until(lambda:matched(rcon.command(f'data get entity UnifiedProbe Inventory[{{Slot:{native_slot}b}}].{name_path}'),'CursorReturnData')),
                        }
                        if version == "1.21.11":
                            result["native_metadata"][str(native_slot)]["nested"] = until(lambda:matched(rcon.command(f'data get entity UnifiedProbe Inventory[{{Slot:{native_slot}b}}].components."minecraft:bundle_contents"[0].components."minecraft:custom_data".CursorNested'),r'\b31\b'))
                result["no_drop"] = until(lambda:matched(rcon.command("execute unless entity @e[type=minecraft:item]"),"Test passed"))
                result["barrel_empty_closed"] = until(lambda:matched(rcon.command("execute if block 0 65 2 minecraft:barrel[open=false] run data get block 0 65 2 Items"),r"\[\]$"))
                result["position_after"] = rcon.command("data get entity UnifiedProbe Pos")
                result["rotation_after"] = rcon.command("data get entity UnifiedProbe Rotation")
                if result["position_before"] != result["position_after"] or result["rotation_before"] != result["rotation_after"]:
                    raise RuntimeError("cursor return changed native pose")
                result["frames"] = [f for f in trace.since(boundary) if f["direction"] == "serverbound" and f["phase"] == "play"]
                clicks = [f for f in result["frames"] if f["packet_id"] == (0x09 if version == "1.16.1" else 0x11)]
                closes = [f for f in result["frames"] if f["packet_id"] == (0x0a if version == "1.16.1" else 0x12)]
                if len(clicks) != (2 if item == "minecraft:stone" else 1) or len(closes) != 1:
                    raise RuntimeError("native cursor return frames missing/duplicated")
                result["authority_limits"] = "Common Client returns held cursor through original PICKUP, waits for each actual source/cursor receipt (plus legacy transaction), then writes exactly one CLOSE. Read-only original frames and independent RCON verify exact counts, no drops, empty closed barrel and unchanged position/rotation. Native disposal audit is separate."
                # Restore the original audit player's fixture for following runs.
                result["fixture"]["clear_after"] = rcon.command("clear UnifiedProbe")
                command = ("replaceitem entity UnifiedProbe inventory.0 minecraft:dirt 2" if version == "1.16.1" else "item replace entity UnifiedProbe inventory.0 with minecraft:dirt 2")
                result["fixture"][command] = rcon.command(command)
                trace.expect_disconnect()
                stage(probe,messages,"cursor_close_audit_disconnect",report["container_records"])
                probe.wait(timeout=10)
                if probe.returncode != 0:
                    raise RuntimeError("native cursor return probe failed after disconnect")
        forced_close_results = {}
        report["native_results"]["native_cursor_close_audit"] = forced_close_results
        report["cursor_close_audit_drop_cleanup"] = rcon.command("kill @e[type=minecraft:item]")
        for mode in ("survival", "creative"):
            # A forced native close does not necessarily establish a new
            # received empty cursor/player UI. Use a fresh real connection,
            # not a guessed cursor or a bypass of the common open guard.
            until(lambda:matched(rcon.command("execute unless entity @a[name=UnifiedProbe]"),"Test passed"))
            probe = subprocess.Popen([str(REPO / "target/debug/examples/common_native_probe")], cwd=REPO, env=dict(env, VOXRIG_NATIVE_SCENARIO="container"), stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=stderr_log, text=True, bufsize=1)
            messages = queue.Queue()
            thread = threading.Thread(target=pump,args=(probe.stdout,messages,probe_log),daemon=True)
            thread.start()
            stage(probe,messages,"container_ready",report["container_records"])
            report["cursor_close_audit_mode_" + mode] = rcon.command("gamemode " + mode + " UnifiedProbe")
            report["cursor_close_audit_teleport_" + mode] = rcon.command("tp UnifiedProbe 0.5 65 0.5 0 35")
            item_fixture = "replaceitem block 0 65 2 container.0 minecraft:stone 5" if version == "1.16.1" else "item replace block 0 65 2 container.0 with minecraft:stone 5"
            report["cursor_close_audit_fixture_" + mode] = rcon.command(item_fixture)
            stage(probe,messages,"cursor_close_audit_open_" + mode,report["container_records"])
            opened = stage(probe,messages,"cursor_close_audit_opened",report["container_records"])["value"]
            stage(probe,messages,"cursor_close_audit_pickup",report["container_records"])
            held = stage(probe,messages,"cursor_close_audit_holding",report["container_records"])["value"]
            audit = {"held_cursor_actual": held}
            forced_close_results[mode] = audit
            boundary = trace.mark()
            report["cursor_close_audit_far_" + mode] = rcon.command("tp UnifiedProbe -7.5 65 -7.5 0 0")
            closed = stage(probe,messages,"cursor_close_audit_forced",report["container_records"])["value"]
            audit["native_forced_close_snapshot"] = closed
            window = opened["observed_screen"]["id"]["window"]
            incoming_id = 0x13 if version == "1.16.1" else 0x11
            outgoing_id = 0x0a if version == "1.16.1" else 0x12
            native_frames = trace.since(boundary)
            close_frames = [f for f in native_frames if f["direction"] == "clientbound" and f["phase"] == "play" and f["packet_id"] == incoming_id and f.get("body_hex") == bytes([window]).hex()]
            audit["actual_original_close_frames"] = close_frames
            if not close_frames:
                raise RuntimeError("actual original native forced-close packet missing from read-only trace")
            if any(f["direction"] == "serverbound" and f["phase"] == "play" and f["packet_id"] == outgoing_id for f in native_frames):
                raise RuntimeError("forced-close audit unexpectedly sent a client close")
            if version == "1.16.1":
                # The isolated world contains no other stone drops. The item can
                # fall away from the teleported creative player before RCON
                # observes it; location is not part of the disposal assertion.
                item = until(lambda:matched(rcon.command('execute as @e[type=minecraft:item,nbt={Item:{id:"minecraft:stone",Count:5b}},limit=1] run data get entity @s'),r'(?s)(?=.*minecraft:stone)(?=.*Count: 5b(?:,|\s|})).*'))
                inventory_after = until(lambda:inventory_matches({9:("minecraft:dirt",2)}))
                report["cursor_close_audit_remove_drop_" + mode] = rcon.command("kill @e[type=minecraft:item]")
            else:
                inventory_after = until(lambda:inventory_matches({9:("minecraft:dirt",2),0:("minecraft:stone",5)}))
                item = until(lambda:matched(rcon.command("execute unless entity @e[type=minecraft:item]"),"Test passed"))
                report["cursor_close_audit_remove_return_" + mode] = rcon.command("clear UnifiedProbe minecraft:stone")
            audit.update(native_inventory=inventory_after, native_item_entity=item)
            trace.expect_disconnect()
            stage(probe,messages,"cursor_close_audit_disconnect",report["container_records"])
            probe.wait(timeout=10)
            if probe.returncode != 0:
                raise RuntimeError("native forced-close audit failed after disconnect")
            forced_close_results[mode] = {"held_cursor_actual":held,"native_forced_close_snapshot":closed,"actual_original_close_frames":close_frames,"native_inventory":inventory_after,"native_item_entity":item,"authority_limits":"Server closes original menu when native range fails after an external fixture teleport; read-only trace forwards exact original compressed bytes and records real incoming CLOSE, with no client close submission. Independent RCON distinguishes native legacy dropped cursor from modern returned cursor. This is a native disposal audit, not implementation or completion of common cursor-bearing close."}
        report["native_results"]["native_cursor_close_audit"] = forced_close_results
        table_results = {}
        report["native_results"]["crafting_table_lifecycle"] = table_results
        for mode in ("survival", "creative"):
            until(lambda:matched(rcon.command("execute unless entity @a[name=UnifiedProbe]"),"Test passed"))
            rcon.command("setblock 0 65 2 minecraft:crafting_table")
            rcon.command("kill @e[type=minecraft:item]")
            probe = subprocess.Popen([str(REPO / "target/debug/examples/common_native_probe")], cwd=REPO, env=dict(env, VOXRIG_NATIVE_SCENARIO="container"), stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=stderr_log, text=True, bufsize=1)
            messages = queue.Queue()
            threading.Thread(target=pump,args=(probe.stdout,messages,probe_log),daemon=True).start()
            stage(probe,messages,"container_ready",report["container_records"])
            table = {"fixture":{}}
            table_results[mode] = table
            for name, command in {
                "mode":"gamemode " + mode + " UnifiedProbe",
                "position":"tp UnifiedProbe 0.5 65 0.5 0 35",
                "clear":"clear UnifiedProbe",
                "material":"replaceitem entity UnifiedProbe inventory.0 minecraft:oak_planks 3" if version=="1.16.1" else "item replace entity UnifiedProbe inventory.0 with minecraft:oak_planks 3",
            }.items():table["fixture"][name]=rcon.command(command)
            def table_baseline():
                player=stage(probe,messages,"armor_fixture_state",report["container_records"])["value"]
                slot=player["inventory"]["slots"][9]
                if player["game_mode"]!=mode or slot is None or slot["source"]["kind"]!="received":return None
                value=slot["value"]
                return player if value["kind"]=="item" and value["item"]["name"]=="minecraft:oak_planks" and value["item"]["count"]==3 else None
            table["received_before"]=until(table_baseline)
            table["native_before"]=until(lambda:inventory_matches({9:("minecraft:oak_planks",3)}))
            table["position_before"]=rcon.command("data get entity UnifiedProbe Pos")
            recipe_results = report["native_results"].setdefault("recipe_catalogue", {})
            catalogue = {"fixture": {}}
            recipe_results[mode] = catalogue
            catalogue["fixture"]["grant"] = rcon.command("recipe give UnifiedProbe *")
            catalogue["received"] = stage(probe,messages,"recipe_catalogue",report["container_records"])["value"]
            preflight_boundary = trace.mark()
            catalogue["materials"] = stage(probe,messages,"recipe_materials",report["container_records"])["value"]
            catalogue["fixture"]["named_material"] = rcon.command(
                "replaceitem entity UnifiedProbe inventory.0 minecraft:oak_planks{display:{Name:'\"RecipeBookNamed\"'}} 3"
                if version == "1.16.1" else
                'item replace entity UnifiedProbe inventory.0 with minecraft:oak_planks[minecraft:custom_name={text:"RecipeBookNamed"}] 3')
            catalogue["named_materials"] = stage(probe,messages,"recipe_materials_named",report["container_records"])["value"]
            catalogue["fixture"]["restored_material"] = rcon.command(
                "replaceitem entity UnifiedProbe inventory.0 minecraft:oak_planks 3" if version == "1.16.1" else
                "item replace entity UnifiedProbe inventory.0 with minecraft:oak_planks 3")
            catalogue["restored_materials"] = stage(probe,messages,"recipe_materials_restored",report["container_records"])["value"]
            for name, named in [("materials",False),("named_materials",True),("restored_materials",False)]:
                facts = catalogue[name]
                for key, amount in [("placement_next","next"),("placement_maximum","maximum")]:
                    plan = facts[key]
                    if (plan["amount"] != amount or plan["mode"] != mode
                        or plan["recipe"] != facts["single"]["recipe"]
                        or plan["receive_sequence"] != facts["context_sequence"]
                        or plan["material_maximum"] != (0 if named else 1)
                        or plan["requested_crafts"] != (0 if named else 1)
                        or plan["source_data_safe"] != (not named)
                        or plan["grid_return"]["remaining"] or plan["grid_return"]["unreturned_splits"]):
                        raise RuntimeError("common coherent recipe placement plan differs from native fixture")
            catalogue["preflight_serverbound_packet_ids"] = [f["packet_id"] for f in trace.since(preflight_boundary) if f["phase"] == "play" and f["direction"] == "serverbound"]
            if (0x19 if version == "1.16.1" else 0x26) in catalogue["preflight_serverbound_packet_ids"]:
                raise RuntimeError("read-only recipe preflight dispatched a recipe request")
            catalogue["fixture"]["revoke"] = rcon.command("recipe take UnifiedProbe minecraft:stick")
            catalogue["removed"] = stage(probe,messages,"recipe_removed",report["container_records"])["value"]
            catalogue["fixture"]["regrant"] = rcon.command("recipe give UnifiedProbe minecraft:stick")
            catalogue["readded"] = stage(probe,messages,"recipe_readded",report["container_records"])["value"]
            catalogue["authority_limits"] = "Actual declaration/display/book receipts and tags captured through identical public Client calls in both versions. Native RCON grants, revokes and regrants stick; Inventory-only material assignment and named-stack exclusion are also captured at one native boundary; no grid placement or predicted consumption is claimed."
            boundary=trace.mark()
            table["open"]=stage(probe,messages,"table_open_"+mode,report["container_records"])["value"]
            table["opened"]=stage(probe,messages,"table_observed_"+mode,report["container_records"])["value"]
            table["filled"]=stage(probe,messages,"table_fill_"+mode,report["container_records"])["value"]
            prediction = table["filled"]["grid_return_plan"]
            if not prediction["remaining"] == [] or prediction["source"] != table["filled"]["grid"]["source"]:
                raise RuntimeError("table return capacity prediction lacks actual opening identity")
            if prediction["unreturned_splits"] or prediction["selected_hotbar"] != {"value":2,"source":{"kind":"submitted"}}:
                raise RuntimeError("table return selection basis is not explicitly submitted")
            table["native_selected_slot"]=until(lambda:matched(rcon.command("data get entity UnifiedProbe SelectedItemSlot"),r": 2$"))
            predicted_slot = next(v for index,v in prediction["predictions"] if index == 9)
            if predicted_slot["source"]["kind"] != "predicted" or predicted_slot["value"]["item"]["count"] != 2:
                raise RuntimeError("grid return prediction includes cursor or is mislabeled as received")
            table["native_filled"]=until(lambda:inventory_matches({9:("minecraft:oak_planks",1)}))
            table["close"]=stage(probe,messages,"table_close_"+mode,report["container_records"])["value"]
            table["after_close"]=stage(probe,messages,"table_after_close_"+mode,report["container_records"])["value"]
            table["native_returned"]=until(lambda:inventory_matches({9:("minecraft:oak_planks",3)}))
            table["return_prediction_evidence"] = "Received inventory has one plank, grid one and cursor one. Grid-only return prediction is two, explicitly Predicted; native close separately returns cursor and grid, producing three actual planks verified by RCON without drops."
            table["native_no_drop"]=until(lambda:matched(rcon.command("execute unless entity @e[type=minecraft:item]"),"Test passed"))
            table["frames"]= [f for f in trace.since(boundary) if f["phase"]=="play"]
            click_id=0x09 if version=="1.16.1" else 0x11
            close_id=0x0a if version=="1.16.1" else 0x12
            outgoing=[f for f in table["frames"] if f["direction"]=="serverbound"]
            if len([f for f in outgoing if f["packet_id"]==click_id])!=4 or len([f for f in outgoing if f["packet_id"]==close_id])!=1:
                raise RuntimeError("table lifecycle lacks three ingredient clicks, one actual cursor-return click and one close")
            table["reopen"]=stage(probe,messages,"table_reopen_"+mode,report["container_records"])["value"]
            table["reopened"]=stage(probe,messages,"table_observed_"+mode,report["container_records"])["value"]
            stale_boundary=trace.mark()
            table["stale_refusal"]=stage(probe,messages,"table_stale_refusal_"+mode,report["container_records"])["value"]
            table["stale_frames"]=[f for f in trace.since(stale_boundary) if f["phase"]=="play"]
            if any(f["direction"]=="serverbound" and f["packet_id"] in (click_id,close_id) for f in table["stale_frames"]):
                raise RuntimeError("stale table handle sent inventory mutation or close")
            empty_boundary=trace.mark()
            table["empty_close"]=stage(probe,messages,"table_close_empty_"+mode,report["container_records"])["value"]
            def empty_close_recorded():
                frames=[f for f in trace.since(empty_boundary) if f["phase"]=="play"]
                return frames if any(f["direction"]=="serverbound" and f["packet_id"]==close_id for f in frames) else None
            # Complete client write may precede the forwarding thread's trace
            # append. Wait for the real outgoing frame, never a fabricated ACK.
            table["empty_close_frames"]=until(empty_close_recorded)
            if len([f for f in table["empty_close_frames"] if f["direction"]=="serverbound" and f["packet_id"]==close_id])!=1:
                raise RuntimeError("empty table close did not write exactly one close")
            table["native_final"]=until(lambda:inventory_matches({9:("minecraft:oak_planks",3)}))
            table["position_after"]=rcon.command("data get entity UnifiedProbe Pos")
            if table["position_after"]!=table["position_before"]:raise RuntimeError("table lifecycle moved native player")
            table["authority_limits"]="Same public Client/mode handles receive native table OPEN/full/cursor/modern processing, place one input in 3x3, observe displayed result, return carried cursor through actual player-slot receipts, and dispatch one close. Fresh player receipt and independent RCON verify native ingredient return with space/living player and no drops. Reopening changes opaque identity; stale input/close requests write nothing. Native empty close is separately dispatched. No result take, recipe consumption/remainder, full-inventory/death disposal guarantee or fabricated close ACK."
            result_takes = report["native_results"].setdefault("crafting_result_take", {})
            takes = {"fixture":{}}
            result_takes[mode] = takes
            def fixture_item(slot, item, count):
                return rcon.command(f"replaceitem entity UnifiedProbe inventory.{slot} minecraft:{item} {count}" if version=="1.16.1" else f"item replace entity UnifiedProbe inventory.{slot} with minecraft:{item} {count}")
            takes["fixture"]["sticks_clear"]=rcon.command("clear UnifiedProbe")
            takes["fixture"]["sticks_material"]=fixture_item(0,"oak_planks",4)
            def result_baseline(expected):
                player=stage(probe,messages,"armor_fixture_state",report["container_records"])["value"]
                for canonical,(item,count) in expected.items():
                    slot=player["inventory"]["slots"][canonical]
                    if slot is None or slot["source"]["kind"]!="received" or slot["value"]["kind"]!="item":return None
                    actual=slot["value"]["item"]
                    if actual["name"]!="minecraft:"+item or actual["count"]!=count:return None
                return player
            takes["sticks_received_before"]=until(lambda:result_baseline({9:("oak_planks",4)}))
            take_boundary=trace.mark()
            takes["sticks"]=stage(probe,messages,"result_sticks_"+mode,report["container_records"])["value"]
            takes["sticks_native"]=until(lambda:inventory_matches({9:("minecraft:oak_planks",2),10:("minecraft:stick",4)}))
            takes["sticks_frames"]=[f for f in trace.since(take_boundary) if f["phase"]=="play"]
            if len([f for f in takes["sticks_frames"] if f["direction"]=="serverbound" and f["packet_id"]==click_id])!=11:
                raise RuntimeError("sticks lacks ten ordinary clicks and one result take; stale refusal must send nothing")
            takes["fixture"]["cake_clear"]=rcon.command("clear UnifiedProbe")
            ingredients={9:("milk_bucket",1),10:("milk_bucket",1),11:("milk_bucket",1),12:("sugar",2),13:("egg",1),14:("wheat",3)}
            for canonical,(item,count) in ingredients.items():takes["fixture"]["cake_"+str(canonical)]=fixture_item(canonical-9,item,count)
            takes["cake_received_before"]=until(lambda:result_baseline(ingredients))
            cake_boundary=trace.mark()
            takes["cake_open"]=stage(probe,messages,"table_reopen_"+mode,report["container_records"])["value"]
            takes["cake_opened"]=stage(probe,messages,"table_observed_"+mode,report["container_records"])["value"]
            takes["cake"]=stage(probe,messages,"result_cake_"+mode,report["container_records"])["value"]
            takes["cake_close"]=stage(probe,messages,"table_close_"+mode,report["container_records"])["value"]
            # Native input disposal chooses the free inventory slot. Audit total
            # counts rather than manufacturing a local destination prediction.
            def cake_inventory():
                response=rcon.command("data get entity UnifiedProbe Inventory")
                stacks=outer_snbt_compounds(response)
                totals={}
                for stack in stacks:
                    item=re.search(r'id: "([^"]+)"',stack)
                    count=re.search(r'(?:Count|count): (\d+)(?:b)?(?:,|\s|})',stack)
                    if not item or not count:return None
                    totals[item.group(1)]=totals.get(item.group(1),0)+int(count.group(1))
                return response if totals=={"minecraft:cake":1,"minecraft:bucket":3} else None
            takes["cake_native"]=until(cake_inventory)
            takes["cake_no_drop"]=until(lambda:matched(rcon.command("execute unless entity @e[type=minecraft:item]"),"Test passed"))
            takes["cake_frames"]=[f for f in trace.since(cake_boundary) if f["phase"]=="play"]
            if len([f for f in takes["cake_frames"] if f["direction"]=="serverbound" and f["packet_id"]==click_id])!=17:
                raise RuntimeError("cake lacks fifteen ingredient clicks, one result take and one cursor return")
            takes["authority_limits"]="Same common Client/mode handles take one 2x2 sticks result that regenerates, consume each input once, and take one 3x3 cake with three actual bucket remainders. Whole fresh native grid/result, matching actual output cursor, native legacy reply and independent RCON totals are checked. Stale take refuses without frames. Recipe selection/planning, nonempty-cursor result merging, shift-crafting and disposal without available space remain outside this evidence."
            trace.expect_disconnect()
            stage(probe,messages,"container_disconnect",report["container_records"])
            probe.wait(timeout=10)
            if probe.returncode!=0:raise RuntimeError("table lifecycle probe failed after disconnect")
        report["scenario_result"] = "passed"
        print(version, "native scenario verified; waiting for clean shutdown", flush=True)
    except BaseException as error:
        report["result"] = "failed"
        report["scenario_result"] = "failed"
        report["error"] = str(error)
        print(version, "FAILED", str(error), flush=True)
        raise
    finally:
        if probe is not None and probe.poll() is None:
            probe.terminate()
            try:
                probe.wait(timeout=10)
            except subprocess.TimeoutExpired:
                probe.kill(); probe.wait(timeout=5)
        if rcon is not None:
            try:
                rcon.command("stop")
            except (OSError, RuntimeError):
                pass
            rcon.close()
        elif server.poll() is None:
            server.stdin.write("stop\n"); server.stdin.flush()
        if server.stdin is not None:
            server.stdin.close()
        try:
            server.wait(timeout=5)
        except subprocess.TimeoutExpired:
            # A live JVM can explain a delayed shutdown. Preserve its own stack
            # dump before considering bounded escalation for this disposable world.
            try:
                server.send_signal(signal.SIGQUIT)
            except ProcessLookupError:
                pass
            try:
                server.wait(timeout=40)
            except subprocess.TimeoutExpired:
                server.terminate()
                try:
                    server.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    server.kill(); server.wait(timeout=5)
        trace.close()
        report["packet_trace"] = {"records": len(trace.frames), "errors": trace.errors,
            "error_contexts":trace.error_contexts,"terminal_events":trace.terminal_events,
            "terminal_deliveries":trace.terminal_deliveries,
            "authority_limits": "Read-only byte-for-byte forwarding of original protocol frames; decompression only for diagnostics, no game method/packet replacement. Trace errors remain failures; terminal context does not fabricate a complete frame."}
        if trace.errors and report["scenario_result"] == "passed":
            report["scenario_result"] = "failed"
            report["result"] = "failed"
            report["error"] = "native packet trace error: " + repr(trace.errors)
        report["server_exit_code"] = server.returncode
        if report["scenario_result"] == "passed":
            report["result"] = "passed" if server.returncode == 0 else "failed"
            if server.returncode != 0:
                report["error"] = "native scenario passed but server did not exit cleanly"
        (folder / "report.json").write_text(json.dumps(report, indent=2) + "\n")
        for handle in (server_log, stderr_log, probe_log):
            handle.close()
        if folder != retained:
            # Export only after the JVM is reaped. No server I/O or gameplay is
            # running while copying the disposable runtime back to diagnostics.
            shutil.copytree(folder, retained)
            shutil.rmtree(folder)
        print(version, "stopped", server.returncode, flush=True)
    if report["result"] != "passed":
        raise RuntimeError(report["error"])
    print(version, "NATIVE_COMMON_CLIENT_PASSED", flush=True)
    return folder


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--version", choices=VERSIONS)
    parser.add_argument("--all", action="store_true")
    parser.add_argument("--accept-eula", action="store_true")
    parser.add_argument("--runtime-dir", type=Path, help="Optional disposable runtime parent, e.g. /dev/shm for isolating disk I/O; reports are exported to .local after JVM exit")
    parser.add_argument("--scenario", choices=("full", "basic-workflow", "equipment-entity", "mining-recovery", "mining-tools", "connection-revocation", "recipe-placement", "recipe-result-merge", "recipe-result-transfer", "recipe-ghost", "recording-scene", "manager-ui", "furnace", "vehicle", "vehicle-control", "dry-terrain", "creative-flight", "creative-landing"), default="full", help="Run the operation corpus or a focused common Client workflow")
    args = parser.parse_args()
    if bool(args.version) == args.all:
        parser.error("select exactly one of --version / --all")
    # Snapshot actual source/data before compilation; do not reconstruct a
    # successful run's inputs from a later worktree or a rebuilt consumer.
    paths = subprocess.check_output([
        "git", "ls-files", "--cached", "--others", "--exclude-standard", "-z", "--", "Cargo.toml", "Cargo.lock", "src", "data",
        "examples/common_native_probe.rs", "scripts/run_common_native.py",
    ], cwd=REPO).decode().rstrip("\0").split("\0")
    runtime_inputs = {
        "baseline_commit": subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPO).decode().strip(),
        "source_sha256": {path: hashlib.sha256((REPO / path).read_bytes()).hexdigest() for path in paths},
    }
    subprocess.run(["cargo", "build", "--locked", "-j1", "--example", "common_native_probe"], cwd=REPO, env=dict(os.environ, CARGO_BUILD_JOBS="1"), check=True)
    if any(hashlib.sha256((REPO / path).read_bytes()).hexdigest() != digest
           for path, digest in runtime_inputs["source_sha256"].items()):
        raise RuntimeError("native inputs changed during build")
    runtime_inputs["consumer_binary_sha256"] = hashlib.sha256(
        (REPO / "target/debug/examples/common_native_probe").read_bytes()).hexdigest()
    for version in VERSIONS if args.all else [args.version]:
        folder = run(version, args.accept_eula,
            args.runtime_dir.resolve() if args.runtime_dir else None, runtime_inputs, args.scenario)
        retained = ROOT / folder.name
        report = json.loads((retained / "report.json").read_text())
        if report["result"] != "passed":
            raise RuntimeError(report.get("error", "native trial failed"))
        print(version, args.scenario, "PASSED", retained, flush=True)


if __name__ == "__main__":
    main()
