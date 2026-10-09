#!/usr/bin/env python3
"""Qualify session-owned one-shot ground requests on isolated official servers."""
import argparse
import hashlib
from pathlib import Path
import re
import struct
import subprocess
import time

from run_climbing_control import REPO, run


def check(version, command, request, trace, report):
    report["synthetic_fixture_not_historical_reproduction"] = True

    def prepare(y=65):
        command(f"tp ClimbingProbe 0.5 {y} 0.5 37 -12")
        request("prepare", position=[0.5, float(y), 0.5])
        return request("start")

    def held(jump=False):
        return dict(forward=1, strafe=0, jump=jump, sneak=True,
                    sprint=False, yaw=37.0, pitch=-12.0)

    def native(record):
        text = command("data get entity ClimbingProbe Pos")
        match = re.search(r"\[([^]]+)\]", text)
        if not match:
            raise RuntimeError("missing independent native position")
        position = [float(v.strip().rstrip("df")) for v in match[1].split(",")]
        if max(abs(a-b) for a,b in zip(position, record["frame"]["position"])) > 0.4:
            raise RuntimeError("native/model position differs")
        return position

    start = prepare()
    controls = held()
    request("keys", controls=controls)
    grounded = request("ticks", count=3)
    if not grounded["frame"]["on_ground"]:
        raise RuntimeError("fixture did not establish model ground")
    rejected = request("ground_jump", session_id=start["session_id"]+999, expect_rejected=True)
    if rejected["kind"] != "State":
        raise RuntimeError("stale session did not reject without input mutation")
    boundary = trace.mark()
    queued = request("ground_jump", session_id=start["session_id"])
    final = request("ticks", count=26)
    if queued["status"]["status"] != "queued" or final["controls"] != controls:
        raise RuntimeError("request did not preserve held input")
    outcome = final["ground_jump"]["status"]
    if outcome["status"] != "predicted" or outcome["outcome"] != "applied":
        raise RuntimeError("dry-ground request was not applied to the model")
    frames = trace.since(boundary)
    moves = [f for f in frames if f["phase"] == "play" and f["direction"] == "serverbound"
             and f["packet_id"] == (0x13 if version == "1.16.1" else 0x1e)]
    positions = [struct.unpack(">ddd", bytes.fromhex(f["body_hex"])[:24]) for f in moves]
    previous, rising, launches = 65.0, False, 0
    for position in positions:
        next_rising = position[1] > previous + 1e-5
        if next_rising and not rising:
            launches += 1
        rising, previous = next_rising, position[1]
    if launches != 1 or max(p[1] for p in positions) <= 65.2 or not final["frame"]["on_ground"]:
        raise RuntimeError("wire traversal did not contain exactly one jump and landing")
    inputs = [f for f in frames if f["phase"] == "play" and f["direction"] == "serverbound"
              and f["packet_id"] == 0x2a]
    if version == "1.21.11":
        bits = [int(f["body_hex"], 16) for f in inputs]
        if sum(bool(b & 16) for b in bits) != 1 or not bits or bits[-1] & 16:
            raise RuntimeError("modern one-shot input was not followed by held-key-preserving release")
        if any((b & 33) != 33 for b in bits):
            raise RuntimeError("forward/sneak input was released by the one-shot request")
    report["checks"].append(dict(name="single_ground_jump_preserves_forward_sneak", queued=queued,
        record=final, native_position=native(final), original_frames=frames, launches=launches,
        stale_rejection=rejected))
    stopped = request("stop")
    rejected = request("ground_jump", session_id=start["session_id"], expect_rejected=True)
    report["checks"].append(dict(name="stopped_request_rejected", record=stopped, rejection=rejected))

    start = prepare(69)
    airborne = request("ticks", count=3)
    if airborne["frame"]["on_ground"]:
        raise RuntimeError("airborne fixture still has model ground")
    queued = request("ground_jump", session_id=start["session_id"])
    landed = request("ticks", count=30)
    if landed["ground_jump"]["status"]["outcome"] != "not_on_ground":
        raise RuntimeError("airborne request was not consumed without a jump")
    if not landed["frame"]["on_ground"] or abs(landed["frame"]["position"][1]-65) > 1e-6:
        raise RuntimeError("airborne request retried on landing")
    report["checks"].append(dict(name="airborne_request_never_retries_on_landing", queued=queued,
        record=landed, native_position=native(landed)))
    request("stop")

    command("fill -4 65 -4 4 65 4 minecraft:water[level=0]")
    start = prepare()
    wet = request("ticks", count=6)
    if not wet["frame"]["in_water"] or not wet["frame"]["on_ground"]:
        raise RuntimeError("shallow-water fixture did not establish model ground")
    queued = request("ground_jump", session_id=start["session_id"])
    wet = request("ticks", count=2)
    if wet["ground_jump"]["status"]["outcome"] != "applied" or wet["controls"]["jump"]:
        raise RuntimeError("grounded water did not use one ground request")
    if wet["frame"]["position"][1] <= 65.1:
        raise RuntimeError("grounded water request did not rise")
    report["checks"].append(dict(name="grounded_water_request_without_held_swim", queued=queued,
        record=wet,native_position=native(wet)))
    request("ticks",count=30)
    request("stop")

    command("fill -4 65 -4 4 67 4 minecraft:water[level=0]")
    start = prepare()
    controls = held(jump=True)
    request("keys", controls=controls)
    wet = request("ticks", count=3)
    if not wet["frame"]["in_water"]:
        raise RuntimeError("held-water fixture did not establish immersion")
    queued = request("ground_jump", session_id=start["session_id"])
    wet = request("ticks", count=6)
    if wet["controls"] != controls or wet["ground_jump"]["status"]["outcome"] != "jump_input_already_held":
        raise RuntimeError("one-shot request changed held swimming input")
    report["checks"].append(dict(name="held_water_input_preserved", queued=queued, record=wet,
        native_position=native(wet)))
    request("stop")
    command("gamemode creative ClimbingProbe")
    deadline = time.monotonic()+5
    while request("player")["game_mode"] not in ("creative", "Creative"):
        if time.monotonic() > deadline:
            raise RuntimeError("creative mode receipt missing")
        time.sleep(0.02)
    rejected = request("ground_jump", session_id=start["session_id"], expect_rejected=True)
    report["checks"].append(dict(name="received_other_mode_rejected", rejection=rejected))
    trace.expect_disconnect()
    request("disconnect")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--accept-eula", action="store_true", required=True)
    parser.add_argument("--version", choices=("1.16.1", "1.21.11"), action="append")
    parser.add_argument("--binary", type=Path, default=REPO / "target/debug/examples/climbing_control_probe")
    parser.add_argument("--jars", type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve()
    revision = subprocess.check_output(["git","rev-parse","HEAD"],cwd=REPO,text=True).strip()
    def recorded_check(*arguments):
        report = arguments[-1]
        report["source_revision"] = revision
        report["source_diff_sha256"] = hashlib.sha256(subprocess.check_output(["git","diff","HEAD"],cwd=REPO)).hexdigest()
        report["binary_sha256"] = hashlib.sha256(binary.read_bytes()).hexdigest()
        report["cargo_lock_sha256"] = hashlib.sha256((REPO/"Cargo.lock").read_bytes()).hexdigest()
        check(*arguments)
    for version in args.version or ("1.16.1", "1.21.11"):
        run(version, binary, args.jars.resolve() if args.jars else None, check=recorded_check)


if __name__ == "__main__":
    main()
