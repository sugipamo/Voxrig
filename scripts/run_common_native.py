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


def run(version, accept_eula, runtime_root=None):
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
    report = {"version":version, "run_id":folder.name, "source":source, "server_memory_limit":"1024M", "sync_chunk_writes":False, "result":"running", "scenario_result":"running", "native_results":{}, "client_records":[]}
    report["runtime_parent"] = str(folder.parent)
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
            response = rcon.command("data get entity UnifiedProbe Inventory")
            stacks = re.findall(r"\{[^{}]*\}", response)
            if len(stacks) != len(expected):
                return None
            for slot, (item, count) in expected.items():
                if not any(re.search(rf"Slot: {slot}b(?:,|\s|}})", stack) and f'id: "{item}"' in stack and re.search(rf"(?:Count|count): {count}(?:b)?(?:,|\s|}})", stack) for stack in stacks):
                    return None
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
        stage(probe,messages,"container_disconnect",report["container_records"])
        probe.wait(timeout=10)
        if probe.returncode != 0:
            raise RuntimeError("container probe failed after disconnect")
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
    subprocess.run(["cargo", "build", "--locked", "-j1", "--example", "common_native_probe"], cwd=REPO, env=dict(os.environ, CARGO_BUILD_JOBS="1"), check=True)
    for version in VERSIONS if args.all else [args.version]:
        run(version, args.accept_eula, args.runtime_dir.resolve() if args.runtime_dir else None)


if __name__ == "__main__":
    main()
