#!/usr/bin/env python3
"""Qualify common control stop/restart on isolated official Java servers.

Uses the common-only climbing probe, original wire frames and independent RCON
positions. This is a synthetic fixture, not a replay of a downstream saved world.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import struct
import subprocess

from run_climbing_control import REPO, run


def check(version, command, request, trace, report):
    report["synthetic_fixture_not_historical_reproduction"] = True

    def prepare(position, rotation):
        command("tp ClimbingProbe " + " ".join(map(str, position + rotation)))
        return request("prepare", position=position)

    def keys(**changes):
        return request("keys", controls=dict(
            forward=changes.get("forward", 0), strafe=0,
            jump=False, sneak=False, sprint=changes.get("sprint", False),
            yaw=37.0, pitch=-12.0))

    def capture(name, record, **extra):
        if record["status"]["status"] != "running":
            raise RuntimeError(name + ": controller not running")
        response = command("data get entity ClimbingProbe Pos")
        match = re.search(r"\[([^]]+)\]", response)
        if not match:
            raise RuntimeError("missing independent native position: " + response)
        native = [float(v.strip().rstrip("df")) for v in match[1].split(",")]
        if max(abs(a-b) for a, b in zip(native, record["frame"]["position"])) > 0.4:
            raise RuntimeError(name + ": native/model position differs")
        report["checks"].append(dict(name=name, record=record, native_position=native, **extra))

    def restart(name, stopped):
        boundary = trace.mark()
        initial = request("start")
        expected = dict(forward=0, strafe=0, jump=False, sneak=False,
                        sprint=False, yaw=37.0, pitch=-12.0)
        if initial["controls"] != expected:
            raise RuntimeError(name + ": restart changed aim or inherited keys")
        if initial["corrections"] or initial["velocity_updates"]:
            raise RuntimeError(name + ": initial receipt boundary counted as a new update")
        record = request("ticks", count=2)
        frames = trace.since(boundary)
        movements = [f for f in frames if f["phase"] == "play" and
                     f["direction"] == "serverbound" and
                     f["packet_id"] == (0x13 if version == "1.16.1" else 0x1e)]
        if not movements:
            raise RuntimeError(name + ": no original movement frame")
        for frame in movements:
            rotation = struct.unpack(">ff", bytes.fromhex(frame["body_hex"])[24:32])
            if rotation != (37.0, -12.0):
                raise RuntimeError(name + ": movement frame reset rotation")
        capture(name, record, initial=initial, stopped=stopped, original_frames=frames)
        return record

    prepare([0.5, 65.0, 0.5], [37.0, -12.0])
    request("start")
    keys(forward=1, sprint=True)
    request("ticks", count=6)
    stopped = request("stop")
    before = stopped["frame"]
    resumed = restart("ground_restart", stopped)
    speed = lambda f: sum(v*v for v in [f["velocity"][0], f["velocity"][2]]) ** 0.5
    if not 0.0 < speed(resumed["frame"]) < speed(before):
        raise RuntimeError("ground restart did not retain decaying model momentum")
    request("stop")

    # The modern shared engine deliberately refuses fall-reset sweeps at >=1
    # block/tick. Qualify landing inside that supported range; below we also
    # assert the existing refusal on a larger drop instead of weakening it.
    prepare([0.5, 75.0 if version == "1.16.1" else 69.0, 0.5], [37.0, -12.0])
    request("start")
    request("ticks", count=3)
    stopped = request("stop")
    before = stopped["frame"]
    if before["on_ground"] or before["velocity"][1] >= 0:
        raise RuntimeError("falling fixture did not establish downward model velocity")
    resumed = restart("falling_restart", stopped)
    after = resumed["frame"]
    if after["on_ground"] or after["velocity"][1] >= before["velocity"][1]:
        raise RuntimeError("falling restart reset downward momentum")
    if after["position"][1] >= before["position"][1]:
        raise RuntimeError("falling restart did not continue falling")
    landed = request("ticks", count=40)
    if not landed["frame"]["on_ground"] or abs(landed["frame"]["position"][1] - 65.0) > 1e-6:
        raise RuntimeError("resumed falling model did not land on the fixture floor")
    capture("resumed_landing", landed)
    request("stop")

    # A new actual pose while stopped replaces the old model/aim once. It is
    # part of the next start boundary, rather than a fabricated velocity ACK.
    received = prepare([2.5, 65.0, 2.5], [81.0, -7.0])
    initial = request("start")
    if [initial["controls"]["yaw"], initial["controls"]["pitch"]] != [81.0, -7.0]:
        raise RuntimeError("new stopped-boundary correction did not replace aim")
    final = request("ticks", count=3)
    if final["corrections"] or final["velocity_updates"]:
        raise RuntimeError("stopped-boundary receipts applied repeatedly")
    if max(abs(a-b) for a,b in zip(final["frame"]["position"], [2.5, 65.0, 2.5])) > 1e-6:
        raise RuntimeError("new stopped-boundary pose was replaced by an old model")
    capture("new_correction_before_restart", final, received=received, initial=initial)
    stopped = request("stop")
    if request("wait", ms=200) != stopped:
        raise RuntimeError("stopped control record changed after release")
    report["checks"].append(dict(name="retained_stopped_record", record=stopped))
    if version == "1.21.11":
        prepare([0.5, 75.0, 0.5], [37.0, -12.0])
        request("start")
        request("ticks", count=3)
        stopped = request("stop")
        request("start")
        paused = request("wait", ms=1200)
        if paused["status"] != dict(status="paused", reason="fall-distance reset sweep"):
            raise RuntimeError("out-of-scope fall reset sweep did not retain its refusal")
        if request("wait", ms=200) != paused:
            raise RuntimeError("unsupported fall sweep kept submitting movement")
        report["checks"].append(dict(name="unsupported_high_fall_remains_paused", record=paused, stopped=stopped))
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
    binary = args.binary.resolve()
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPO, text=True).strip()
    source_diff = subprocess.check_output(["git", "diff", "HEAD"], cwd=REPO)

    def recorded_check(*arguments):
        report = arguments[-1]
        report["source_revision"] = revision
        report["source_diff_sha256"] = hashlib.sha256(source_diff).hexdigest()
        report["binary_sha256"] = hashlib.sha256(binary.read_bytes()).hexdigest()
        report["cargo_lock_sha256"] = hashlib.sha256((REPO / "Cargo.lock").read_bytes()).hexdigest()
        check(*arguments)

    for version in args.version or ("1.16.1", "1.21.11"):
        run(version, binary, args.jars.resolve() if args.jars else None, check=recorded_check)


if __name__ == "__main__":
    main()
