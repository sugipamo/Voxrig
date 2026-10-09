#!/usr/bin/env python3
"""Validate common continuous climbing on disposable offline vanilla servers.

Requires Java, the built climbing_control_probe, and explicit --accept-eula.
RCON supplies independent server positions; the proxy forwards original bytes.
Artifacts and credentials stay in the ignored .local directory. No public bind.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import queue
import re
import secrets
import struct
import subprocess
import sys
import threading
import time

sys.dont_write_bytecode = True

from run_common_native import PacketTraceProxy, Rcon, download, free_port, pump, until

REPO = Path(__file__).resolve().parents[1]
ROOT = REPO / ".local/climbing/live"


def run_vehicle_checks(version, vehicle, mode, command, request, trace, report):
    """Original interaction/mount, actual motion, released inputs and actual exit."""
    command("gamemode " + mode + " ClimbingProbe")
    command("tp ClimbingProbe 0.5 65 -0.5 0 0")
    entity = "minecraft:minecart" if vehicle == "minecart" else (
        "minecraft:boat" if version == "1.16.1" else "minecraft:oak_boat")
    if vehicle == "boat":
        command("fill -12 62 -12 12 62 12 minecraft:stone")
        command("fill -12 63 -12 12 64 12 minecraft:water[level=0]")
        command("summon " + entity + ' 0.5 64.65 1.5 {Tags:["ControlMount"],Invulnerable:1b}')
    else:
        command("fill 0 64 0 0 64 12 minecraft:stone")
        command("fill 0 65 0 0 65 2 minecraft:rail[shape=north_south]")
        command("fill 0 65 3 0 65 10 minecraft:powered_rail[shape=north_south,powered=false]")
        command("summon " + entity + ' 0.5 65.1 1.5 {Tags:["ControlMount"],NoGravity:1b,Invulnerable:1b}')
    request("prepare", position=[0.5, 65.0, -0.5])
    mounted = request("mount", type=entity)
    report["checks"].append(dict(name=vehicle + "_mount", received=mounted,
                                native_mount=command("data get entity ClimbingProbe RootVehicle.Attach")))

    def native(field):
        response = command("data get entity @e[tag=ControlMount,limit=1] " + field)
        match = re.search(r"\[([^]]+)\]", response)
        if not match:
            raise RuntimeError("missing native vehicle field: " + response)
        return [float(v.strip().rstrip("df")) for v in match[1].split(",")]

    def drive(name, forward, strafe, active, neutral):
        initial = native("Pos")
        inputs = [dict(forward=forward, strafe=strafe, jump=False)] * active
        inputs += [dict(forward=0, strafe=0, jump=False)] * neutral
        boundary = trace.mark()
        record = request("drive", inputs=inputs)
        def complete_trace():
            frames = trace.since(boundary)
            moves = [f for f in frames if f["phase"] == "play" and f["direction"] == "serverbound"
                     and f["packet_id"] == (0x16 if version == "1.16.1" else 0x21)]
            return frames if len(moves) >= len(inputs) else None
        frames = until(complete_trace) if vehicle == "boat" else trace.since(boundary)
        if vehicle == "boat":
            predicted = record["boat_motion"]["frames"][-1]["position"]
            def applied_position():
                actual = native("Pos")
                return actual if max(abs(a-b) for a,b in zip(actual, predicted)) < 1e-6 else None
            after = until(applied_position, 5)
        else:
            after = native("Pos")
        if record["stage"] != "submitted" or record["dispatched_ticks"] != len(inputs):
            raise RuntimeError("mounted plan did not completely submit")
        evidence = dict(name=name, record=record, native_before=initial, native_after=after,
                        native_rotation=native("Rotation"), frames=frames)
        report["checks"].append(evidence)
        if vehicle == "boat":
            boat = record["boat_motion"]
            predicted = boat["frames"][-1]["position"]
            if max(abs(a-b) for a,b in zip(after, predicted)) > 0.15:
                raise RuntimeError("boat native/predicted position differs")
            if boat["frames"][-1]["paddles"] != [False, False]:
                raise RuntimeError("neutral plan left paddles active")
            movements = [f for f in evidence["frames"] if f["phase"] == "play"
                         and f["direction"] == "serverbound" and f["packet_id"] == (0x16 if version == "1.16.1" else 0x21)]
            paddles = [f for f in evidence["frames"] if f["phase"] == "play"
                       and f["direction"] == "serverbound" and f["packet_id"] == (0x17 if version == "1.16.1" else 0x22)]
            if len(movements) != len(inputs) or len(paddles) != len(inputs):
                raise RuntimeError("boat missing complete original movement/paddle frames")
            for predicted_frame, move, paddle in zip(boat["frames"], movements, paddles):
                expected = struct.pack(">dddff", *predicted_frame["position"], *predicted_frame["rotation"])
                if version == "1.21.11":
                    expected += bytes([predicted_frame["on_ground"]])
                if move.get("body_hex") != expected.hex():
                    raise RuntimeError("boat wire motion differs from the retained frame")
                if paddle.get("body_hex") != bytes(predicted_frame["paddles"]).hex():
                    raise RuntimeError("boat wire paddles differ from the retained frame")
            if paddles[-1].get("body_hex") != "0000":
                raise RuntimeError("boat missing paddle release")
        return initial, after

    initial, after = drive(vehicle + "_forward", 1, 0, 16 if vehicle == "boat" else 32, 8)
    if after[2] <= initial[2] + (0.3 if vehicle == "boat" else 0.05):
        raise RuntimeError("native vehicle did not move forward")
    if vehicle == "boat":
        before, after = drive("boat_turn", 1, 1, 10, 8)
        if after[0] <= before[0] + 0.1:
            raise RuntimeError("native boat did not turn left")
        drive("boat_reverse", -1, 0, 10, 12)
    drive(vehicle + "_neutral", 0, 0, 0, 100)
    before = native("Pos")
    request("wait", ms=500)
    after = native("Pos")
    if max(abs(a-b) for a,b in zip(before, after)) > 0.005:
        raise RuntimeError("released vehicle did not settle")
    if vehicle == "minecart" and max(abs(v) for v in native("Motion")) > 0.005:
        raise RuntimeError("unpowered braking rails did not stop cart")
    complete = request("dismount")
    if complete["stage"] != "completed":
        raise RuntimeError("actual passenger exit not completed")
    until(lambda: "Test passed" in command("execute unless entity @a[name=ClimbingProbe,nbt={RootVehicle:{}}]"))
    report["checks"].append(dict(name=vehicle + "_dismount", record=complete))
    trace.expect_disconnect()
    request("disconnect")


def run(version, binary, jars, vehicle=None, mode="survival", check=None, server_properties=None,
        trace_factory=PacketTraceProxy, server_launcher=None):
    folder = ROOT / (version + "-" + time.strftime("%Y%m%d-%H%M%S"))
    folder.mkdir(parents=True)
    if jars:
        jar = jars / (version + "-server.jar")
        expected = {"1.16.1": "a412fd69db1f81db3f511c1463fd304675244077",
                    "1.21.11": "64bb6d763bed0a9f1d632ec347938594144943ed"}[version]
        if hashlib.sha1(jar.read_bytes()).hexdigest() != expected:
            raise RuntimeError("official server SHA-1 mismatch")
    else:
        jar, _ = download(version)
    port, rcon_port = free_port(), free_port()
    while port == rcon_port:
        rcon_port = free_port()
    password = secrets.token_hex(24)
    props = {
        "server-ip": "127.0.0.1", "server-port": port, "level-name": "world",
        "level-type": "flat", "level-seed": "1234", "online-mode": "false",
        "gamemode": "survival", "force-gamemode": "true", "spawn-protection": 0,
        "view-distance": 2, "simulation-distance": 2, "max-players": 2,
        "max-tick-time": 60000, "enable-rcon": "true", "rcon.port": rcon_port,
        "rcon.password": password, "broadcast-rcon-to-ops": "false",
        "enable-status": "false", "sync-chunk-writes": "false",
        "network-compression-threshold": 256,
    }
    if server_properties:
        if set(server_properties) - {"view-distance", "simulation-distance", "max-players"}:
            raise ValueError("unsupported isolated server property override")
        props.update(server_properties)
    if version != "1.16.1":
        props["generator-settings"] = json.dumps({"biome": "minecraft:plains", "layers": [
            {"block": "minecraft:bedrock", "height": 1},
            {"block": "minecraft:dirt", "height": 2},
            {"block": "minecraft:grass_block", "height": 1}], "structure_overrides": []})
    path = folder / "server.properties"
    path.write_text("".join(f"{k}={v}\n" for k, v in props.items()))
    path.chmod(0o600)
    (folder / "eula.txt").write_text("eula=true\n")
    report = {"version": version, "server_sha1": hashlib.sha1(jar.read_bytes()).hexdigest(),
              "checks": [], "commands": [], "passed": False}
    server = probe = rcon = trace = None
    try:
        with (folder / "server.log").open("w") as log:
            java = ["java", "-XX:ActiveProcessorCount=1", "-Xms256M", "-Xmx1024M"]
            launch = (["-jar", str(jar)] if server_launcher is None else
                      ["--add-opens=java.base/java.lang=ALL-UNNAMED", "-cp",
                       str(server_launcher[0]) + os.pathsep + str(jar), server_launcher[1]])
            server = subprocess.Popen(java + launch + ["nogui"], cwd=folder,
                                      stdin=subprocess.PIPE, stdout=log, stderr=subprocess.STDOUT, text=True)

            def connect():
                if server.poll() is not None:
                    raise RuntimeError("server exited; inspect server.log")
                try:
                    return Rcon(rcon_port, password)
                except OSError:
                    return None

            rcon = until(connect, 120)

            def command(text):
                response = rcon.command(text)
                report["commands"].append({"command": text, "response": response})
                if any(s in response for s in ("Incorrect argument", "Unknown or incomplete", "not loaded")):
                    raise RuntimeError("fixture command rejected: " + text + ": " + response)
                return response

            command("forceload add -8 -8 8 8")
            def loaded():
                command("setblock 0 64 0 minecraft:stone")
                return "Test passed" in command("execute if block 0 64 0 minecraft:stone")
            until(loaded, 30)
            for text in ["fill -8 64 -8 8 64 8 minecraft:stone", "fill -4 65 -4 4 84 4 minecraft:air",
                         "setworldspawn 0 65 0", "difficulty peaceful"]:
                command(text)
            for rule, value in ([('spawnRadius', '0'), ('doMobSpawning', 'false'), ('doDaylightCycle', 'false')]
                                if version == "1.16.1" else [('minecraft:respawn_radius', '0'),
                                ('minecraft:spawn_mobs', 'false'), ('minecraft:advance_time', 'false')]):
                command(f"gamerule {rule} {value}")
            trace = trace_factory(port, version, folder / "packets.jsonl")
            with (folder / "probe.log").open("w") as output, (folder / "probe.stderr.log").open("w") as stderr:
                probe = subprocess.Popen([str(binary)], cwd=REPO, env=dict(os.environ,
                    VOXRIG_MINECRAFT_VERSION=version, VOXRIG_PORT=str(trace.port), VOXRIG_VEHICLE_MODE=mode), stdin=subprocess.PIPE,
                    stdout=subprocess.PIPE, stderr=stderr, text=True, bufsize=1)
                messages = queue.Queue()
                reader = threading.Thread(target=pump, args=(probe.stdout, messages, output), daemon=True)
                reader.start()

                def receive():
                    try:
                        return json.loads(messages.get(timeout=20))
                    except queue.Empty as error:
                        raise RuntimeError("probe timed out; inspect probe.stderr.log") from error

                if receive() != {"ready": True}:
                    raise RuntimeError("missing ready result")

                def request(action, **arguments):
                    probe.stdin.write(json.dumps(dict(command=action, **arguments)) + "\n")
                    probe.stdin.flush()
                    return receive()

                def keys(**changes):
                    return request("keys", controls=dict(forward=changes.get("forward", 0), strafe=0,
                        jump=changes.get("jump", False), sneak=changes.get("sneak", False),
                        sprint=changes.get("sprint", False), yaw=0.0, pitch=0.0))

                def position():
                    text = command("data get entity ClimbingProbe Pos")
                    match = re.search(r"\[([^]]+)\]", text)
                    if not match:
                        raise RuntimeError("missing native position: " + text)
                    return [float(v.strip().removesuffix("d")) for v in match[1].split(",")]

                def capture(name, record):
                    native = position()
                    if record["status"]["status"] != "running":
                        raise RuntimeError(name + " stopped or paused")
                    frame = record["frame"]["position"]
                    # The RCON command can run one server tick after the snapshot.
                    if max(abs(a - b) for a, b in zip(native, frame)) > 0.4:
                        raise RuntimeError(name + " native/predicted position differs")
                    if record["corrections"]:
                        raise RuntimeError(name + " received an unexpected correction")
                    check = dict(name=name, record=record, native_position=native)
                    report["checks"].append(check)
                    return native

                if check:
                    check(version, command, request, trace, report)
                elif vehicle:
                    run_vehicle_checks(version, vehicle, mode, command, request, trace, report)
                else:
                    command("effect give ClimbingProbe minecraft:water_breathing 600 0 true")
                    for terrain, block, wet in [
                        ("ladder", "minecraft:ladder[facing=north,waterlogged=false]", False),
                        ("vine", "minecraft:vine[south=true]", False),
                        ("scaffolding", "minecraft:scaffolding[distance=0,waterlogged=false]", False),
                        ("waterlogged_ladder", "minecraft:ladder[facing=north,waterlogged=true]", True),
                        ("waterlogged_scaffolding", "minecraft:scaffolding[distance=0,waterlogged=true]", True),
                    ]:
                        command("tp ClimbingProbe 3.5 65 3.5 0 0")
                        command("fill -1 65 -1 1 80 2 minecraft:air")
                        if wet:
                            command("fill -1 65 -1 1 79 1 minecraft:stone")
                            command("fill 0 65 0 0 79 0 minecraft:air")
                        elif terrain != "scaffolding":
                            command("fill 0 65 1 0 79 1 minecraft:stone")
                        command("fill 0 65 0 0 78 0 " + block)
                        command("tp ClimbingProbe 0.5 65 0.5 0 0")
                        request("prepare", position=[0.5, 65.0, 0.5])
                        start = request("start")
                        keys(jump=True)
                        rising = request("ticks", count=32)
                        up = capture(terrain + "_jump", rising)
                        if wet and not rising["frame"]["in_water"]:
                            raise RuntimeError(terrain + " did not observe immersion")
                        if up[1] < (66 if wet else 68):
                            raise RuntimeError(terrain + " did not climb")
                        if "scaffolding" not in terrain:
                            keys(forward=1)
                            wall = capture(terrain + "_wall", request("ticks", count=20))
                            if wall[1] < up[1] + 1:
                                raise RuntimeError(terrain + " did not climb against the wall")
                            up = wall
                        keys(sneak=True)
                        hold = capture(terrain + "_sneak", request("ticks", count=12))
                        if wet or terrain == "scaffolding":
                            if hold[1] >= up[1] - 0.3:
                                raise RuntimeError(terrain + " sneak did not descend")
                        elif abs(hold[1] - up[1]) > 0.3:
                            raise RuntimeError(terrain + " sneak did not hold height")
                        stop_mark = trace.mark()
                        stopped = request("stop")
                        retained = request("wait", ms=250)
                        if stopped != retained or stopped["status"]["status"] != "stopped":
                            raise RuntimeError("stopped control kept ticking")
                        frames = trace.since(stop_mark)
                        packet = 0x1c if version == "1.16.1" else 0x2a
                        release = [f for f in frames if f["direction"] == "serverbound" and f["phase"] == "play"
                                   and (f["packet_id"] == packet)]
                        if version == "1.16.1":
                            def action(frame):
                                body = bytes.fromhex(frame["body_hex"])
                                _, offset = PacketTraceProxy.varint(body)
                                return PacketTraceProxy.varint(body[offset:])[0]
                            if not any(action(f) == 1 for f in release):
                                raise RuntimeError("missing legacy sneak release")
                        elif not any(f.get("body_hex") == "00" for f in release):
                            raise RuntimeError("missing modern neutral input release")
                        report["checks"].append(dict(name=terrain + "_stop", start=start,
                                                    record=stopped, release_frames=frames))

                    for facing, opened, passable in [("north", "true", True),
                                                      ("south", "true", False),
                                                      ("north", "false", False)]:
                        command("tp ClimbingProbe 3.5 65 3.5 0 0")
                        command("fill -1 65 -1 1 80 2 minecraft:air")
                        command("fill 0 65 1 0 79 1 minecraft:stone")
                        command("fill 0 65 0 0 78 0 minecraft:ladder[facing=north,waterlogged=false]")
                        command("setblock 0 69 0 minecraft:oak_trapdoor[facing=" + facing
                                + ",half=bottom,open=" + opened + ",powered=false,waterlogged=false]")
                        command("tp ClimbingProbe 0.5 65 0.5 0 0")
                        request("prepare", position=[0.5, 65.0, 0.5])
                        request("start")
                        keys(jump=True)
                        point = capture("trapdoor_" + facing + "_" + opened, request("ticks", count=60))
                        if (point[1] > 70) != passable:
                            raise RuntimeError("trapdoor climbing ignored facing/open state")
                        request("stop")

                    # Fresh session, external teleport, then terminal mode transition while sprinting.
                    command("fill -1 65 -1 1 80 2 minecraft:air")
                    command("tp ClimbingProbe 0.5 65 -2.5 0 0")
                    request("prepare", position=[0.5, 65.0, -2.5])
                    request("start")
                    keys(forward=1, sprint=True)
                    before = capture("restart_sprint", request("ticks", count=8))
                    command("tp ClimbingProbe 0.5 65 -2.5 0 0")
                    record = request("ticks", count=8)
                    if record["corrections"] != 1 or record["status"]["status"] != "running":
                        raise RuntimeError("external teleport did not recover exactly once")
                    report["checks"].append(dict(name="teleport", before=before, record=record,
                                                native_position=position()))
                    boundary = trace.mark()
                    command("gamemode creative ClimbingProbe")
                    terminal = request("wait", ms=200)
                    if terminal["status"]["status"] != "stopped":
                        raise RuntimeError("mode transition did not stop control")
                    frames = trace.since(boundary)
                    report["checks"].append(dict(name="mode_change", record=terminal, frames=frames))
                    commands = [f for f in frames if f["direction"] == "serverbound" and f["phase"] == "play"
                                and f["packet_id"] == (0x1c if version == "1.16.1" else 0x29)]
                    def sprint_stopped(frame):
                        body = bytes.fromhex(frame["body_hex"])
                        _, offset = PacketTraceProxy.varint(body)
                        return PacketTraceProxy.varint(body[offset:])[0] == (4 if version == "1.16.1" else 2)
                    if not any(sprint_stopped(f) for f in commands):
                        raise RuntimeError("automatic stop did not release sprint")
                    if version != "1.16.1" and not any(
                        f["direction"] == "serverbound" and f["phase"] == "play"
                        and f["packet_id"] == 0x2a and f.get("body_hex") == "00" for f in frames
                    ):
                        raise RuntimeError("automatic stop did not release modern input")
                    command("gamemode survival ClimbingProbe")
                    request("wait", ms=150)
                    request("start")
                    keys(jump=True)
                    request("ticks", count=3)
                    trace.expect_disconnect()
                    terminal = request("disconnect")
                    if terminal["status"]["status"] != "stopped":
                        raise RuntimeError("disconnected control retained a running record")
                    report["checks"].append(dict(name="disconnect", record=terminal))
                if probe.wait(timeout=10) != 0:
                    raise RuntimeError("probe failed")
                report["trace_errors"] = trace.errors
                if trace.errors:
                    raise RuntimeError("packet trace errors")
                report["passed"] = True
    except BaseException as error:
        report["error"] = str(error)
        raise
    finally:
        if probe and probe.poll() is None:
            probe.terminate()
            try:
                probe.wait(timeout=5)
            except subprocess.TimeoutExpired:
                probe.kill()
                probe.wait(timeout=5)
        if trace:
            trace.close()
        if server and server.poll() is None:
            try:
                server.stdin.write("stop\n")
                server.stdin.flush()
                server.wait(timeout=20)
            except (OSError, subprocess.TimeoutExpired):
                server.kill()
                server.wait(timeout=5)
        if rcon:
            rcon.close()
        (folder / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({"version": version, "passed": report["passed"], "report": str(folder / "report.json")}))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--accept-eula", action="store_true", required=True)
    parser.add_argument("--version", choices=("1.16.1", "1.21.11"), action="append")
    parser.add_argument("--binary", type=Path, default=REPO / "target/debug/examples/climbing_control_probe")
    parser.add_argument("--vehicle", choices=("boat", "minecart"), help="run a mounted workflow instead of climbing")
    parser.add_argument("--vehicle-mode", choices=("survival", "creative"), default="survival")
    parser.add_argument("--jars", type=Path, help="directory containing SHA-1 verified official server jars")
    args = parser.parse_args()
    for version in args.version or ("1.16.1", "1.21.11"):
        run(version, args.binary.resolve(), args.jars.resolve() if args.jars else None, args.vehicle, args.vehicle_mode)


if __name__ == "__main__":
    main()
