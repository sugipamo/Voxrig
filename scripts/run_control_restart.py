#!/usr/bin/env python3
"""Stop/restart common control against unchanged official loopback servers."""
import argparse
from pathlib import Path
import struct

from run_climbing_control import REPO, run
from run_common_native import until


def check(version, command, request, trace, report):
    command("fill -8 64 -8 8 64 8 minecraft:stone")
    command("fill -8 65 -8 8 80 8 minecraft:air")

    def restart(name, rotation, falling=False):
        boundary = trace.mark()
        result = request("restart")
        old, start, first = (result[k] for k in ("stopped", "started", "first"))
        assert old["status"]["status"] == "stopped"
        assert start["session_id"] != old["session_id"]
        assert [start["controls"][k] for k in ("yaw", "pitch")] == rotation
        assert start["controls"]["forward"] == 0 and not start["controls"]["jump"]
        assert first["dispatched_ticks"] == 1 and first["status"]["status"] == "running"
        assert first["corrections"] == 0 and first["velocity_updates"] == 0
        position = first["frame"]["position"]
        previous = old["frame"]
        if version == "1.21.11":
            # No compatibility idle physics on this version: the next displacement
            # is exactly the previous model momentum, never the initial receipt.
            if name == "moving_restart" or falling:
                for axis in ((0, 1, 2) if falling else (0, 2)):
                    assert abs(position[axis] - previous["position"][axis] - previous["velocity"][axis]) < 1e-10
        if falling:
            assert position[1] < previous["position"][1]
            assert first["frame"]["velocity"][1] < previous["velocity"][1]
        payload = struct.pack(">dddff", *position, *rotation)
        flags = int(first["frame"]["on_ground"])
        if version == "1.21.11":
            flags |= int(first["frame"]["horizontal_collision"]) << 1
        payload += bytes([flags])
        move_id = 0x13 if version == "1.16.1" else 0x1e
        def written():
            return [p for p in trace.since(boundary) if p["phase"] == "play"
                    and p["direction"] == "serverbound" and p["packet_id"] == move_id
                    and p.get("body_hex") == payload.hex()]
        wire = until(written)
        report["checks"].append(dict(name=name, **result, first_position_packet=wire[0]))

    command("tp ClimbingProbe 0.5 65 0.5 37 -12")
    request("prepare", position=[0.5, 65, 0.5])
    start = request("start")
    assert [start["controls"][k] for k in ("yaw", "pitch")] == [37, -12]
    request("keys", controls=dict(forward=1, strafe=0, jump=False, sneak=False,
                                  sprint=False, yaw=37, pitch=-12))
    request("ticks", count=6)
    restart("moving_restart", [37, -12])
    for i in range(3):
        request("ticks", count=2)
        restart(f"released_restart_{i + 1}", [37, -12])
    request("stop")

    command("tp ClimbingProbe 0.5 71 0.5 113 24")
    request("prepare", position=[0.5, 71, 0.5])
    start = request("start")
    assert [start["controls"][k] for k in ("yaw", "pitch")] == [113, 24]
    request("ticks", count=2)
    restart("falling_restart", [113, 24], falling=True)
    landed = request("ticks", count=25)
    assert landed["frame"]["on_ground"] and landed["frame"]["position"][1] == 65
    assert landed["corrections"] == 0
    report["checks"].append(dict(name="falling_lands_without_correction", record=landed,
                                  native_position=command("data get entity ClimbingProbe Pos")))
    request("stop")
    command("tp ClimbingProbe 4.5 65 4.5 151 -23")
    request("prepare", position=[4.5, 65, 4.5])
    start = request("start")
    assert [start["controls"][k] for k in ("yaw", "pitch")] == [151, -23]
    current = request("ticks", count=2)
    assert current["frame"]["position"] == [4.5, 65, 4.5]
    report["checks"].append(dict(name="fresh_pose_after_stop", started=start, record=current))
    request("stop")
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
