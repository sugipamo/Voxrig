#!/usr/bin/env python3
"""Owned common ground requests on unchanged official loopback servers."""
import argparse
from pathlib import Path
import re
import struct

from run_climbing_control import REPO, run
from run_common_native import until


def check(version, command, request, trace, report):
    command("effect give ClimbingProbe minecraft:water_breathing 600 0 true")

    def save(name, **facts):
        report["checks"].append(dict(name=name, **facts))

    def rejected(session):
        result = request("jump", session_id=session)
        assert "error" in result
        return result

    def controls(**changes):
        return dict(forward=changes.get("forward", 0), strafe=0,
                    jump=changes.get("jump", False), sneak=changes.get("sneak", False),
                    sprint=False, yaw=37, pitch=-12)

    def prepare(y=65):
        request("stop")
        command("fill -8 64 -8 8 64 8 minecraft:stone")
        command("fill -8 65 -8 8 80 8 minecraft:air")
        command(f"tp ClimbingProbe 0.5 {y} 0.5 37 -12")
        request("prepare", position=[0.5, y, 0.5])

    def outcome(record, expected):
        result = record["ground_jump"]
        assert result["status"]["status"] == "evaluated"
        assert result["status"]["outcome"] == expected
        assert result["status"]["dispatched"]
        return result

    def movements(boundary):
        move_id = 0x13 if version == "1.16.1" else 0x1e
        return [p for p in trace.since(boundary) if p["phase"] == "play"
                and p["direction"] == "serverbound" and p["packet_id"] == move_id]

    save("absent_session_refused", response=rejected(1))
    prepare()
    session = request("start")["session_id"]
    held = controls(forward=1, sneak=True)
    request("keys", controls=held)
    before = request("ticks", count=4)
    assert before["frame"]["on_ground"]
    stale = rejected(session + 1)
    boundary = trace.mark()
    queued = request("jump", session_id=session)["request"]
    assert queued["status"]["status"] == "queued"
    after = request("ticks", count=32)
    outcome(after, "applied")
    assert after["controls"] == held and after["frame"]["on_ground"]
    assert after["frame"]["position"][1] == 65 and not after["corrections"]
    wire = movements(boundary)
    heights = [65.] + [struct.unpack(">ddd", bytes.fromhex(p["body_hex"])[:24])[1] for p in wire]
    assert max(heights) > 66
    starts = sum(b > a + 1e-8 and (i == 0 or a <= heights[i - 1] + 1e-8)
                 for i, (a, b) in enumerate(zip(heights, heights[1:])))
    assert starts == 1, "a one-shot request jumped repeatedly"
    inputs = [p for p in trace.since(boundary) if p["phase"] == "play"
              and p["direction"] == "serverbound" and p["packet_id"] == 0x2a]
    if version == "1.21.11":
        assert [p["body_hex"] for p in inputs] == ["31", "21"]
    native = command("data get entity ClimbingProbe Pos")
    save("single_ground_jump_preserves_forward_sneak_aim", before=before, queued=queued,
         stale_refusal=stale, after=after, native_position=native,
         movement_packets=wire, input_packets=inputs)
    stopped = request("stop")
    save("stopped_session_refused", record=stopped, response=rejected(session))

    prepare(71)
    session = request("start")["session_id"]
    before = request("ticks", count=2)
    assert not before["frame"]["on_ground"]
    queued = request("jump", session_id=session)["request"]
    after = request("ticks", count=25)
    outcome(after, "airborne")
    assert after["frame"]["on_ground"] and after["frame"]["position"][1] == 65
    assert not after["corrections"]
    save("airborne_consumed_without_landing_retry", before=before, queued=queued, after=after,
         native_position=command("data get entity ClimbingProbe Pos"))
    request("stop")

    if version == "1.21.11":
        prepare()
        session = request("start")["session_id"]
        command("attribute ClimbingProbe minecraft:jump_strength base set 0")
        request("ticks", count=4)
        queued = request("jump", session_id=session)["request"]
        after = request("ticks", count=4)
        outcome(after, "no_jump_power")
        assert after["frame"]["position"][1] == 65
        save("received_zero_jump_strength", queued=queued, after=after)
        command("attribute ClimbingProbe minecraft:jump_strength base set 0.42")
        request("stop")

    for kind in ("water", "ladder"):
        prepare()
        if kind == "water":
            command("fill -3 65 -3 3 72 3 minecraft:water[level=0]")
            command("tp ClimbingProbe 0.5 68 0.5 37 -12")
            request("prepare", position=[0.5, 68, 0.5])
        else:
            command("fill 0 65 -1 0 79 -1 minecraft:stone")
            command("fill 0 65 0 0 79 0 minecraft:ladder[facing=north,waterlogged=false]")
            request("wait", ms=150)
        session = request("start")["session_id"]
        held = controls(jump=True)
        request("keys", controls=held)
        before = request("ticks", count=4)
        boundary = trace.mark()
        queued = request("jump", session_id=session)["request"]
        after = request("ticks", count=5)
        outcome(after, "already_held")
        assert after["controls"] == held and after["frame"]["position"][1] > before["frame"]["position"][1]
        assert not after["corrections"]
        if version == "1.21.11":
            assert not [p for p in trace.since(boundary) if p["phase"] == "play"
                        and p["direction"] == "serverbound" and p["packet_id"] == 0x2a]
        save("held_" + kind + "_input_preserved", before=before, queued=queued, after=after,
             native_position=command("data get entity ClimbingProbe Pos"))
        request("stop")

    prepare()
    command("setblock 0 65 0 minecraft:nether_portal[axis=x]")
    request("wait", ms=150)
    session = request("start")["session_id"]
    request("wait", ms=120)
    paused = request("record")
    assert paused["status"]["status"] == "paused"
    response = rejected(session)
    command("setblock 0 65 0 minecraft:air")
    resumed = request("ticks", count=4)
    assert resumed["ground_jump"] is None and resumed["frame"]["position"][1] == 65
    save("paused_request_refused_without_later_jump", paused=paused, response=response, resumed=resumed)
    request("stop")

    prepare()
    session = request("start")["session_id"]
    request("ticks", count=3)
    queued = request("jump", session_id=session)["request"]
    command("gamemode creative ClimbingProbe")
    request("wait", ms=150)
    stopped = request("record")
    assert stopped["status"]["status"] == "stopped"
    assert stopped["ground_jump"]["status"]["status"] != "queued"
    save("received_mode_change_fences_request", queued=queued, record=stopped, response=rejected(session))
    command("gamemode survival ClimbingProbe")
    request("wait", ms=150)

    prepare()
    old = request("start")["session_id"]
    command("kill ClimbingProbe")
    request("wait", ms=150)
    rejected(old)
    respawn = request("respawn")
    def alive():
        text = command("data get entity ClimbingProbe Health")
        match = re.search(r"([-+0-9.]+)f$", text)
        return text if match and float(match[1]) > 0 else None
    until(alive)
    command("tp ClimbingProbe 0.5 65 0.5 37 -12")
    request("prepare", position=[0.5, 65, 0.5])
    response = rejected(old)
    current = request("start")["session_id"]
    assert current != old
    stale = rejected(old)
    request("ticks", count=3)
    save("respawn_world_and_replaced_session_fenced", respawn=respawn, old_session=old,
         current_session=current, world_refusal=response, stale_refusal=stale)

    queued = request("jump", session_id=current)["request"]
    trace.expect_disconnect()
    revoked = request("revoke_control", session_id=current)
    assert revoked["jump_rejected"] and revoked["record"]["status"]["status"] == "stopped"
    assert revoked["record"]["ground_jump"]["status"]["status"] != "queued"
    save("revoked_connection_retains_request_outcome", queued=queued, result=revoked)


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
