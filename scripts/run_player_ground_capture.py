#!/usr/bin/env python3
"""Validate #25 player ground/rotation capture on disposable vanilla servers."""
import argparse
import struct
from pathlib import Path
from run_climbing_control import run
from run_common_native import until


def check(version, command, request, trace, report):
    movement_id = 0x13 if version == "1.16.1" else 0x1e

    def capture(name, ground=None, missing=False):
        value = request("capture")
        player = value["player"]
        if missing:
            assert player["on_ground"] is None, (name, player)
        else:
            assert player["on_ground"] == dict(value=ground, source=dict(kind="predicted")), (name, player)
            position = player["position"]["value"]
            # Locate the actual original position packet corresponding to this
            # capture, instead of assuming a separately fetched record is coeval.
            def written():
                for frame in trace.since(0):
                    if frame["direction"] == "serverbound" and frame["phase"] == "play" and frame["packet_id"] == movement_id:
                        body = bytes.fromhex(frame["body_hex"])
                        if len(body) == 33 and struct.unpack(">ddd", body[:24]) == tuple(position) and bool(body[-1] & 1) == ground:
                            return frame
                return None
            packet = until(written)
            assert player["rotation_source"] == dict(kind="submitted")
            value["matching_native_frame"] = packet
            native_ground = until(lambda: (response if response.rstrip().endswith("1b" if ground else "0b") else None)
                                  if (response := command("data get entity ClimbingProbe OnGround")) else None)
            value["later_native_on_ground"] = native_ground
            value["later_native_position"] = command("data get entity ClimbingProbe Pos")
        report["checks"].append(dict(name=name, capture=value))
        return player

    command("spawnpoint ClimbingProbe 0 65 0")
    command("tp ClimbingProbe 0.5 65 0.5 0 0")
    request("prepare", position=[0.5, 65.0, 0.5])
    initial = capture("correction_missing_ground", missing=True)
    assert initial["rotation_source"]["kind"] == "received"
    request("start")
    request("ticks", count=3)
    capture("standing_prediction", ground=True)
    request("keys", controls=dict(forward=0, strafe=0, jump=True, sneak=False, sprint=False, yaw=25.0, pitch=5.0))
    def airborne():
        player = request("player")
        return player if player["on_ground"] and not player["on_ground"]["value"] else None
    until(airborne)
    capture("jump_prediction", ground=False)
    request("keys", controls=dict(forward=0, strafe=0, jump=False, sneak=False, sprint=False, yaw=25.0, pitch=5.0))
    request("ticks", count=25)
    capture("landing_prediction", ground=True)
    stop = request("stop")
    stopped = capture("stopped_local_prediction", ground=True)
    report["checks"][-1]["control_record"] = stop
    # Modern stationary look requires a fresh received baseline; continuous
    # predictions alone are not its settled finite-motion admission contract.
    command("tp ClimbingProbe 0.5 65 0.5 25 5")
    request("prepare", position=[0.5, 65.0, 0.5])
    received_rotation = request("player")["received_pose"]["rotation"]
    looked = request("look", rotation=[70.0, 0.0])
    assert looked["rotation"] == [70.0, 0.0]
    assert looked["rotation_source"] == dict(kind="submitted")
    assert looked["received_pose"]["rotation"] == received_rotation
    # PR #26 exposes model ground only, not the submitted stationary-look bit.
    assert looked["on_ground"] is None
    report["checks"].append(dict(name="independent_look_source", player=looked))
    command("tp ClimbingProbe 0.5 65 0.5 70 0")
    request("prepare", position=[0.5, 65.0, 0.5])
    corrected = capture("same_coordinate_correction_clears_ground", missing=True)
    assert corrected["rotation_source"]["kind"] == "received"
    command("kill ClimbingProbe")
    until(lambda: request("player")["health"]["value"]["health"] <= 0)
    request("respawn")
    def new_world():
        player = request("player")
        return player if player["session"]["world_generation"] != corrected["session"]["world_generation"] and player["position"] else None
    respawned = until(new_world)
    assert respawned["session"]["connection_id"] == corrected["session"]["connection_id"]
    report["checks"].append(dict(name="native_respawn_generation", player=respawned))
    command("tp ClimbingProbe 0.5 65 0.5 0 0")
    request("prepare", position=[0.5, 65.0, 0.5])
    capture("respawn_pose_has_no_old_ground", missing=True)
    trace.expect_disconnect()
    revoked = request("revoke")
    assert revoked["player_rejected"]
    report["checks"].append(dict(name="revoke_refuses_new_capture", result=revoked))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--accept-eula", action="store_true", required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--jars", type=Path, required=True)
    parser.add_argument("--version", choices=["1.16.1", "1.21.11"], action="append")
    args = parser.parse_args()
    for version in args.version or ["1.16.1", "1.21.11"]:
        run(version, args.binary.resolve(), args.jars.resolve(), check=check)


if __name__ == "__main__":
    main()
