#!/usr/bin/env python3
"""Common bubble/boat fluid control against disposable unchanged vanilla servers.

RCON independently reads positions/passengers; a transparent proxy records all
original packets. Requires explicit EULA acceptance and the common-only probe.
"""
import argparse
from pathlib import Path
import re
import struct

from run_climbing_control import REPO, run
from run_common_native import until


def check(version, command, request, trace, report):
    command("gamerule " + ("randomTickSpeed" if version == "1.16.1" else "minecraft:random_tick_speed") + " 0")
    command("effect give ClimbingProbe minecraft:water_breathing 600 0 true")

    def native(selector, field="Pos"):
        text = command(f"data get entity {selector} {field}")
        match = re.search(r"\[([^]]+)\]", text)
        if not match:
            raise RuntimeError("missing native vector: " + text)
        return [float(v.strip().rstrip("df")) for v in match[1].split(",")]

    for down in (False, True):
        command("tp ClimbingProbe 3.5 65 3.5 0 0")
        command("fill -1 65 -1 1 80 1 minecraft:stone")
        command("setblock 0 64 0 minecraft:" + ("magma_block" if down else "soul_sand"))
        command("fill 0 65 0 0 79 0 minecraft:bubble_column[drag=" + str(down).lower() + "]")
        command("setblock 0 80 0 minecraft:air")
        y = 77.0 if down else 68.0
        command(f"tp ClimbingProbe 0.5 {y} 0.5 0 0")
        request("prepare", position=[0.5, y, 0.5])
        request("start")
        phases = [("interior", 18)] if down else [("interior", 18), ("surface_exit", 12)]
        for phase, count in phases:
            record = request("ticks", count=count)
            position = native("ClimbingProbe")
            if record["status"]["status"] != "running" or record["corrections"]:
                raise RuntimeError("bubble control paused or received an unexpected correction")
            if max(abs(a - b) for a, b in zip(position, record["frame"]["position"])) > 0.8:
                raise RuntimeError("bubble native/predicted position differs by more than one tick")
            if (down and position[1] >= y - 1.0) or (not down and position[1] <= y + 1.0):
                raise RuntimeError("bubble did not move in the expected direction")
            if phase == "surface_exit" and position[1] <= 80.0:
                raise RuntimeError("upward bubble did not leave the water surface")
            report["checks"].append(dict(name=f"bubble_{'down' if down else 'up'}_{phase}",
                                         record=record, native_position=position))
        stopped = request("stop")
        if stopped["status"]["status"] != "stopped":
            raise RuntimeError("bubble control did not stop")
        report["checks"].append(dict(name=f"bubble_{'down' if down else 'up'}_stop", record=stopped))

    boat_type = "minecraft:boat" if version == "1.16.1" else "minecraft:oak_boat"
    boat_selector = "@e[tag=FluidMount,limit=1]"
    move_id, paddle_id = (0x16, 0x17) if version == "1.16.1" else (0x21, 0x22)

    def wire(record, boundary):
        frames = record["boat_motion"]["frames"]
        def arrived():
            packets = trace.since(boundary)
            moves = [p for p in packets if p["phase"] == "play" and p["direction"] == "serverbound"
                     and p["packet_id"] == move_id]
            paddles = [p for p in packets if p["phase"] == "play" and p["direction"] == "serverbound"
                       and p["packet_id"] == paddle_id]
            return (moves, paddles) if len(moves) >= len(frames) and len(paddles) >= len(frames) else None
        moves, paddles = until(arrived)
        if len(moves) != len(frames) or len(paddles) != len(frames):
            raise RuntimeError("boat packet count differs from completely dispatched ticks")
        for frame, move, paddle in zip(frames, moves, paddles):
            body = struct.pack(">dddff", *frame["position"], *frame["rotation"])
            if version == "1.21.11":
                body += bytes([frame["on_ground"]])
            if move["body_hex"] != body.hex() or paddle["body_hex"] != bytes(frame["paddles"]).hex():
                raise RuntimeError("boat wire bytes differ from the retained prediction")
        return trace.since(boundary)

    for kind in ("source", "flowing", "current", "forced_exit"):
        command("kill @e[tag=FluidMount]")
        command("fill -12 65 -12 12 81 12 minecraft:air")
        command("fill -12 62 -12 12 62 12 minecraft:stone")
        command("fill -12 63 -12 12 64 12 minecraft:water[level=0]")
        command("tp ClimbingProbe 0.5 65 -0.5 0 0")
        command("summon " + boat_type + ' 0.5 64.65 1.5 {Tags:["FluidMount"],Invulnerable:1b}')
        request("prepare", position=[0.5, 65.0, -0.5])
        mounted = request("mount", type=boat_type)
        report["checks"].append(dict(name="boat_" + kind + "_mount", received=mounted,
                                     native_mount=command("data get entity ClimbingProbe RootVehicle.Attach")))
        # Flood a genuinely received mount. A source layer above falling water
        # keeps the downward stream supplied while ordinary scheduled ticks run.
        if kind == "current":
            command("fill -12 65 -12 12 65 0 minecraft:water[level=0]")
            command("fill -12 65 1 12 65 12 minecraft:water[level=1]")
        else:
            command("fill -12 65 -12 12 69 12 minecraft:water[level=" + ("8" if kind == "flowing" else "0") + "]")
        if kind == "flowing":
            command("fill -12 70 -12 12 70 12 minecraft:water[level=0]")
            command("fill -2 63 -12 -2 69 12 minecraft:stone")
        request("wait", ms=100)
        before = native(boat_selector)
        inputs = ([dict(forward=1, strafe=1, jump=False)] * 12
                  + [dict(forward=0, strafe=0, jump=False)] * 8) if kind != "forced_exit" else (
                      [dict(forward=1, strafe=0, jump=False)] * 89
                      + [dict(forward=0, strafe=0, jump=False)])
        if kind == "current":
            inputs = [dict(forward=0, strafe=0, jump=False)] * 20
        boundary = trace.mark()
        if kind == "forced_exit":
            outcome = request("drive_until_interrupted", inputs=inputs)
            if not outcome["error"]:
                raise RuntimeError("forced passenger exit did not return the retained conflict")
            record = outcome["record"]
        else:
            record = request("drive", inputs=inputs)
        packets = wire(record, boundary)
        after = native(boat_selector)
        report["checks"].append(dict(name="boat_" + kind + "_drive", record=record,
                                     native_before=before, native_after=after, packets=packets))
        if kind == "forced_exit":
            if record["stage"] != "requires_inspection" or record["dispatched_ticks"] >= len(inputs):
                raise RuntimeError("server-forced passenger exit did not interrupt dispatch")
            observed = request("vehicle")
            if observed["relation"]["value"]["kind"] != "unmounted":
                raise RuntimeError("forced passenger exit was not actually received")
            if "Test passed" not in command("execute unless entity @a[name=ClimbingProbe,nbt={RootVehicle:{}}]"):
                raise RuntimeError("native passenger relationship remains mounted")
            request("wait", ms=200)
            mark = trace.mark()
            request("wait", ms=250)
            if any(p["phase"] == "play" and p["direction"] == "serverbound" and p["packet_id"] in (move_id, paddle_id)
                   for p in trace.since(mark)):
                raise RuntimeError("boat frames continued after the received passenger exit")
            report["checks"].append(dict(name="boat_forced_exit_received", observed=observed,
                                         retained=request("vehicle_record")))
        else:
            if record["stage"] != "submitted" or record["requires_inspection"] is not None:
                raise RuntimeError("water boat control did not completely submit")
            frames = record["boat_motion"]["frames"]
            expected = "under_flowing_water" if kind in ("flowing", "current") else "under_water"
            if not any(f["status"] == expected for f in frames):
                raise RuntimeError("boat did not enter the expected water status")
            if max(abs(a - b) for a, b in zip(after, frames[-1]["position"])) > 0.4:
                raise RuntimeError("water boat native/predicted position differs")
            if max(abs(after[i] - before[i]) for i in (0, 2)) < 0.1:
                raise RuntimeError("submerged boat did not steer")
            if kind == "current" and (after[2] <= before[2] + 0.2 or any(f["paddles"] != [False, False] for f in frames)):
                raise RuntimeError("released boat was not carried by the horizontal current")
            if frames[-1]["paddles"] != [False, False]:
                raise RuntimeError("final neutral did not release paddles")
            complete = request("dismount")
            if complete["stage"] != "completed":
                raise RuntimeError("explicit water boat exit did not complete")
            report["checks"].append(dict(name="boat_" + kind + "_dismount", record=complete))

    trace.expect_disconnect()
    request("disconnect")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--accept-eula", action="store_true", required=True)
    parser.add_argument("--version", choices=("1.16.1", "1.21.11"), action="append")
    parser.add_argument("--binary", type=Path, default=REPO / "target/debug/examples/climbing_control_probe")
    parser.add_argument("--jars", type=Path)
    args = parser.parse_args()
    for version in args.version or ("1.16.1", "1.21.11"):
        run(version, args.binary.resolve(), args.jars.resolve() if args.jars else None, check=check)


if __name__ == "__main__":
    main()
