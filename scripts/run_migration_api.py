#!/usr/bin/env python3
"""Check #28/#29 through common-only inputs on disposable official servers."""
import argparse
import math
import re
import struct
from pathlib import Path
from run_climbing_control import run
from run_common_native import until


def check(version, command, request, trace, report):
    look_id, slot_id = (0x13, 0x24) if version == "1.16.1" else (0x1f, 0x34)

    def ask(action, **arguments):
        value = request(action, **arguments)
        assert value["ok"], value
        return value["result"]

    def refused(action, kind, **arguments):
        value = request(action, **arguments)
        assert not value["ok"] and value["kind"] == kind, value
        return value

    def ray(name, origin, direction, distance):
        value = ask("raycast", origin=origin, direction=direction, distance=distance)
        report["checks"].append(dict(name=name, raycast=value))
        return value["result"]

    def wait_hit(origin, direction, distance, position):
        def received():
            result = ask("raycast", origin=origin, direction=direction, distance=distance)["result"]
            return result if isinstance(result, dict) and result.get("Hit", {}).get("position") == position else None
        return until(received)

    command("forceload add 0 0 64 0")
    command("fill 0 66 0 64 67 0 minecraft:air")
    command("tp ClimbingProbe 0.5 65 0.5 0 0")
    ask("prepare", position=[0.5, 65.0, 0.5])
    ask("loaded", min=[0, 66, 0], max=[64, 67, 0])
    command("setblock 40 66 0 minecraft:stone")
    wait_hit([0.5, 66.75, 0.5], [1, 0, 0], 48, [40, 66, 0])
    hit = ray("obstruction_beyond_32", [0.5, 66.75, 0.5], [1, 0, 0], 48)["Hit"]
    assert hit["position"] == [40, 66, 0] and hit["distance"] == 39.5
    command("setblock 40 66 0 minecraft:air")
    command("setblock 42 66 0 minecraft:torch")
    command("setblock 46 66 0 minecraft:oak_slab[type=bottom,waterlogged=false]")
    until(lambda: ask("raycast", origin=[0.5, 66.75, 0.5], direction=[1, 0, 0], distance=48)["result"] == "Miss")
    assert ray("transparent_path_over_partial_shape", [0.5, 66.75, 0.5], [1, 0, 0], 48) == "Miss"
    wait_hit([0.5, 66.25, 0.5], [1, 0, 0], 48, [46, 66, 0])
    hit = ray("partial_shape_beyond_32", [0.5, 66.25, 0.5], [1, 0, 0], 48)["Hit"]
    assert hit["position"] == [46, 66, 0] and hit["distance"] == 45.5
    command("setblock 48 66 0 minecraft:stone")
    direction = [48, 0, 0.25]
    distance = math.hypot(48, 0.25)
    wait_hit([0.5, 66.75, 0.5], direction, distance, [48, 66, 0])
    hit = ray("48_plus_target_offset", [0.5, 66.75, 0.5], direction, distance)["Hit"]
    assert hit["position"] == [48, 66, 0] and 47.5 < hit["distance"] < distance
    command("setblock 48 66 0 minecraft:air")
    command("setblock 64 66 0 minecraft:stone")
    wait_hit([0.5, 66.75, 0.5], [1, 0, 0], 64, [64, 66, 0])
    hit = ray("maximum_64", [0.5, 66.75, 0.5], [1, 0, 0], 64)["Hit"]
    assert hit["position"] == [64, 66, 0] and hit["distance"] == 63.5
    value = refused("raycast", "InvalidInput", origin=[0.5, 66.75, 0.5], direction=[1, 0, 0], distance=64.001)
    report["checks"].append(dict(name="distance_bound_refused", result=value))
    chunks = ask("chunks")["chunks"]
    boundary = (max(x for x, z in chunks if z == 0) + 1) * 16
    origin = [boundary - 40.5, 70.75, 0.5]
    missing = ray("unloaded_boundary_beyond_32", origin, [1, 0, 0], 48)["Unloaded"]
    assert missing["position"] == [boundary, 70, 0], missing
    assert boundary - origin[0] > 32

    def modes(mode):
        command("gamemode " + mode + " ClimbingProbe")
        until(lambda: ask("player")["game_mode"] == mode)
        command("tp ClimbingProbe 0.5 65 0.5 0 0")
        return ask("prepare", position=[0.5, 65.0, 0.5])

    def originals(boundary, packet):
        return [f for f in trace.since(boundary) if f["phase"] == "play"
                and f["direction"] == "serverbound" and f["packet_id"] == packet]

    def native_rotation():
        response = command("data get entity ClimbingProbe Rotation")
        values = re.search(r"\[([^]]+)\]", response)
        return [float(v.strip().rstrip("f")) for v in values[1].split(",")]

    for mode in ["survival", "creative", "adventure", "spectator"]:
        before = modes(mode)
        boundary = trace.mark()
        looked = ask("look", mode=mode, rotation=[37.0, -12.0])
        assert looked["receipt"]["interaction_sequence"] is None
        assert looked["player"]["rotation_source"] == dict(kind="submitted")
        assert looked["player"]["received_pose"]["rotation"] == before["received_pose"]["rotation"]
        def matched():
            frames = originals(boundary, look_id)
            return next((f for f in frames if bytes.fromhex(f["body_hex"])[-9:-1] == struct.pack(">ff", 37, -12)), None)
        frame = until(matched)
        until(lambda: native_rotation() == [37.0, -12.0])
        report["checks"].append(dict(name=mode + "_look", result=looked, original=frame, later_server_rotation=native_rotation()))
        boundary = trace.mark()
        if mode == "spectator":
            selected = refused("hotbar", "Unsupported", mode=mode, slot=8)
            assert not originals(boundary, slot_id)
        else:
            selected = ask("hotbar", mode=mode, slot=8)
            assert selected["player"]["selected_hotbar"] == dict(value=8, source=dict(kind="submitted"))
            until(lambda: any(f["body_hex"] == "0008" for f in originals(boundary, slot_id)))
            until(lambda: command("data get entity ClimbingProbe SelectedItemSlot").rstrip().endswith("8"))
        report["checks"].append(dict(name=mode + "_hotbar", result=selected, originals=originals(boundary, slot_id)))
        refused("look", "InvalidInput", mode=mode, rotation=[123.0, 90.1])
        refused("hotbar", "InvalidInput", mode=mode, slot=9)
        report["checks"].append(dict(name=mode + "_invalid_inputs_refused"))

    # Change the actual received mode, then retain the previously expected mode.
    modes("adventure")
    boundary = trace.mark()
    look = refused("look", "State", mode="spectator", rotation=[87.0, 12.0])
    slot = refused("hotbar", "State", mode="spectator", slot=4)
    assert not originals(boundary, slot_id)
    assert not any(bytes.fromhex(f["body_hex"])[-9:-1] == struct.pack(">ff", 87, 12) for f in originals(boundary, look_id))
    report["checks"].append(dict(name="received_mode_change_refused", look=look, hotbar=slot))
    before = ask("raycast", origin=[0.5, 70.75, 0.5], direction=[1, 0, 0], distance=48)
    command("kill ClimbingProbe")
    until(lambda: ask("player")["health"]["value"]["health"] <= 0)
    ask("respawn")
    until(lambda: ask("player")["session"]["world_generation"] != before["session"]["world_generation"])
    command("tp ClimbingProbe 0.5 65 0.5 0 0")
    ask("prepare", position=[0.5, 65.0, 0.5])
    after = ask("raycast", origin=[0.5, 70.75, 0.5], direction=[1, 0, 0], distance=48)
    assert after["session"]["connection_id"] == before["session"]["connection_id"]
    assert after["session"]["world_generation"] != before["session"]["world_generation"]
    report["checks"].append(dict(name="raycast_respawn_world_boundary", before=before, after=after))
    trace.expect_disconnect()
    ask("disconnect")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--accept-eula", action="store_true", required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--jars", type=Path, required=True)
    parser.add_argument("--version", choices=["1.16.1", "1.21.11"], action="append")
    args = parser.parse_args()
    for version in args.version or ["1.16.1", "1.21.11"]:
        run(version, args.binary.resolve(), args.jars.resolve(), check=check, server_properties={"view-distance": 6})


if __name__ == "__main__":
    main()
