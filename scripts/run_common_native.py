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

    def expect_disconnect(self):
        """Scope terminal delivery diagnostics to the actual requested connection."""
        with self.lock:
            if not self.connection_states:
                raise RuntimeError("disconnect requested without a traced connection")
            state = self.connection_states[-1]
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
    for name, observation in state["registries"].items():
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
        if state["registries"] or snapshot["unbreaking"] is not None or state["legacy_codec"] is None:
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
    if name not in ("ready", "mining_ready", "placement_ready", "swap_ready", "container_ready"):
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


def run(version, accept_eula, runtime_root=None, runtime_inputs=None):
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
gamemode=creative
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
            result["fixture"]["clear_after"] = rcon.command("clear UnifiedProbe")
            result["fixture"]["restore"] = rcon.command("replaceitem entity UnifiedProbe inventory.0 minecraft:dirt 2" if version == "1.16.1" else "item replace entity UnifiedProbe inventory.0 with minecraft:dirt 2")
        trace.expect_disconnect()
        stage(probe,messages,"container_disconnect",report["container_records"])
        probe.wait(timeout=10)
        if probe.returncode != 0:
            raise RuntimeError("container probe failed after disconnect")
        cursor_returns = {}
        report["native_results"]["cursor_return_close"] = cursor_returns
        return_items = [("minecraft:stone", 5), ("minecraft:diamond_helmet", 1)]
        if version == "1.21.11":
            return_items.append(("minecraft:bundle", 1))
        for mode in ("survival", "creative"):
            for item, count in return_items:
                until(lambda:matched(rcon.command("execute unless entity @a[name=UnifiedProbe]"),"Test passed"))
                probe = subprocess.Popen([str(REPO / "target/debug/examples/common_native_probe")], cwd=REPO, env=dict(env, VOXRIG_NATIVE_SCENARIO="container"), stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=stderr_log, text=True, bufsize=1)
                messages = queue.Queue()
                thread = threading.Thread(target=pump,args=(probe.stdout,messages,probe_log),daemon=True)
                thread.start()
                stage(probe,messages,"container_ready",report["container_records"])
                key = mode + "/" + item
                result = {"fixture": {}}
                cursor_returns[key] = result
                for command in ("clear UnifiedProbe", "kill @e[type=minecraft:item]", "gamemode " + mode + " UnifiedProbe", "tp UnifiedProbe 0.5 65 0.5 0 35"):
                    result["fixture"][command] = rcon.command(command)
                if item == "minecraft:stone":
                    command = ("replaceitem entity UnifiedProbe inventory.0 minecraft:stone 63" if version == "1.16.1" else "item replace entity UnifiedProbe inventory.0 with minecraft:stone 63")
                    result["fixture"][command] = rcon.command(command)
                command = (f"replaceitem block 0 65 2 container.0 {item} {count}" if version == "1.16.1" else f"item replace block 0 65 2 container.0 with {item} {count}")
                result["fixture"][command] = rcon.command(command)
                stage(probe,messages,"cursor_close_audit_open_" + mode,report["container_records"])
                result["opened"] = stage(probe,messages,"cursor_close_audit_opened",report["container_records"])["value"]
                stage(probe,messages,"cursor_close_audit_pickup",report["container_records"])
                result["held"] = stage(probe,messages,"cursor_return_holding",report["container_records"])["value"]
                result["position_before"] = rcon.command("data get entity UnifiedProbe Pos")
                result["rotation_before"] = rcon.command("data get entity UnifiedProbe Rotation")
                boundary = trace.mark()
                result["close"] = stage(probe,messages,"cursor_return_close",report["container_records"])["value"]
                result["inventory_after"] = until(lambda:inventory_matches({9:(item,64),10:(item,4)} if item == "minecraft:stone" else {9:(item,1)}))
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
                item = until(lambda:matched(rcon.command("execute at UnifiedProbe as @e[type=minecraft:item,distance=..3,limit=1,sort=nearest] run data get entity @s"),r'(?s)(?=.*minecraft:stone)(?=.*Count: 5b(?:,|\s|})).*'))
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
    args = parser.parse_args()
    if bool(args.version) == args.all:
        parser.error("select exactly one of --version / --all")
    # Snapshot actual source/data before compilation; do not reconstruct a
    # successful run's inputs from a later worktree or a rebuilt consumer.
    paths = subprocess.check_output([
        "git", "ls-files", "-z", "--", "Cargo.toml", "Cargo.lock", "src", "data",
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
        run(version, args.accept_eula, args.runtime_dir.resolve() if args.runtime_dir else None, runtime_inputs)


if __name__ == "__main__":
    main()
