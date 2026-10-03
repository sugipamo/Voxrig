#!/usr/bin/env python3
"""Run a fixed common-Client scenario against isolated, sequential vanilla servers.

Results come from native server RCON independently of Voxrig's received cache.
Only disposable files under .local/native-client-unification are modified.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import queue
import re
import secrets
import signal
import socket
import struct
import subprocess
import threading
import time
import urllib.request
import uuid

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


def stage(probe, messages, name, records, timeout=30, poll=None):
    if name != "ready":
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
        if record["stage"] == name or (name == "disconnect" and record["stage"] == "disconnected"):
            print("native", name, "received", flush=True)
            return record
    raise TimeoutError("probe stage timed out: " + name)


def run(version, accept_eula):
    if not accept_eula:
        raise RuntimeError("pass --accept-eula when authorized to run the official server")
    jar, source = download(version)
    folder = ROOT / ("trial-" + version + "-" + uuid.uuid4().hex[:8])
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
    report = {"version":version, "run_id":folder.name, "source":source, "server_memory_limit":"1024M", "sync_chunk_writes":False, "result":"running", "scenario_result":"running", "native_results":{}, "client_records":[]}
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
        env = dict(os.environ, VOXRIG_MINECRAFT_VERSION=version, VOXRIG_PORT=str(port))
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
        report["preview_teleport"] = rcon.command("tp UnifiedProbe 0.5 65 0.5 0 0")
        stationary = until(lambda: matched(rcon.command("data get entity UnifiedProbe Pos"), r"\[0\.5d, 65\.0d, 0\.5d\]"))
        stage(probe, messages, "survival_preview", report["client_records"])
        after_preview = rcon.command("data get entity UnifiedProbe Pos")
        if after_preview != stationary:
            raise RuntimeError("read-only preview changed the native stationary position")
        report["native_results"]["preview_position_before"] = stationary
        report["native_results"]["preview_position_after"] = after_preview
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
        stage(probe, messages, "disconnect", report["client_records"])
        probe.wait(timeout=10)
        if probe.returncode != 0:
            raise RuntimeError("probe failed after disconnect")
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
        report["server_exit_code"] = server.returncode
        if report["scenario_result"] == "passed":
            report["result"] = "passed" if server.returncode == 0 else "failed"
            if server.returncode != 0:
                report["error"] = "native scenario passed but server did not exit cleanly"
        (folder / "report.json").write_text(json.dumps(report, indent=2) + "\n")
        for handle in (server_log, stderr_log, probe_log):
            handle.close()
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
    args = parser.parse_args()
    if bool(args.version) == args.all:
        parser.error("select exactly one of --version / --all")
    subprocess.run(["cargo", "build", "--locked", "-j1", "--example", "common_native_probe"], cwd=REPO, env=dict(os.environ, CARGO_BUILD_JOBS="1"), check=True)
    for version in VERSIONS if args.all else [args.version]:
        run(version, args.accept_eula)


if __name__ == "__main__":
    main()
