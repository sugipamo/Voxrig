#!/usr/bin/env python3
"""Qualify a server-processed no-effect inventory click on both official servers.

The timing gate holds one unchanged original click while the official server
switches the player to spectator. Vanilla then processes the click without
applying it: 1.21.11 resends full contents (sendAllDataToRemote) and still
answers the fence; 1.16.1 refreshes the container without a transaction reply,
so the send needs an explicit resync. No packet is injected, rewritten or
replayed. The received boundary must equal the pre-send inventory.
"""
import argparse
import json
import sys
from pathlib import Path

sys.dont_write_bytecode = True

import run_climbing_control as harness
from run_common_native import until
from run_inventory_click_receipts import DelayedClickTrace


def check(version, command, request, trace, report):
    legacy = version == "1.16.1"
    report["timing_fixture"] = ("One original serverbound click delayed while RCON sets spectator; "
                                "no rewriting, injection or replay.")

    def count(name, slot=9):
        value = request("player")["inventory"]["slots"][slot]
        return (value["value"]["item"]["count"] if value and value["value"]["kind"] == "item"
                and value["value"]["item"]["name"] == "minecraft:" + name else 0)

    def mode(name):
        return lambda: request("player")["game_mode"] == name

    def status(sent, *names):
        def ready():
            result = request("inventory_send_record")
            assert result["record"]["id"] == sent["id"], result
            return result if result["record"]["status"] in names else None
        return ready

    command("tp ClimbingProbe 0.5 65 0.5 0 0")
    request("prepare", position=[0.5, 65.0, 0.5])
    command("clear ClimbingProbe")
    command("replaceitem entity ClimbingProbe inventory.0 minecraft:cobblestone 3"
            if legacy else
            "item replace entity ClimbingProbe inventory.0 with minecraft:cobblestone 3")
    until(lambda: count("cobblestone") == 3, 5)
    before = request("player")
    assert before["inventory"]["cursor"]["value"]["kind"] == "empty", before
    baseline = request("inventory_changes")
    boundary = trace.mark()
    trace.armed = True
    trace.release.clear()
    trace.held.clear()
    try:
        sent = request("send_inventory_click", slot=9, button="left")
        assert trace.held.wait(2), "original click did not reach timing gate"
        command("gamemode spectator ClimbingProbe")
        until(mode("spectator"), 5)
    finally:
        trace.release.set()
    resync = None
    if legacy:
        # 1.16.1 spectator branch: refreshContainer only, no ConfirmTransaction.
        expired = until(status(sent, "resync_required"), 15)
        command("gamemode survival ClimbingProbe")
        until(mode("survival"), 5)
        resync = request("resync_inventory")
        done = until(status(resync, "received", "diverged"), 5)
    else:
        expired = None
        done = until(status(sent, "received", "diverged"), 5)
        command("gamemode survival ClimbingProbe")
        until(mode("survival"), 5)
    assert done["record"]["status"] == "diverged", done
    divergence = done["divergence"]
    assert done["effect"] == "no_effect", done
    assert divergence["send"] == sent["id"], divergence
    assert divergence["actual"]["slots"] == [] and divergence["actual"]["items"] == [], divergence
    kinds = [cause["kind"] for cause in divergence["causes"]]
    if legacy:
        assert "timed_out_resync" in kinds and "external" not in kinds, divergence
    else:
        # Previously the fallback labelled this own no-op as External.
        assert kinds == [], divergence
    assert count("cobblestone") == 3, request("player")
    native = command("data get entity ClimbingProbe Inventory")
    assert "minecraft:cobblestone" in native, native
    log = request("inventory_changes", after=baseline["receive_sequence"])
    assert log["changes"] == [], log
    strict = request("inventory_record")
    clicks = [frame for frame in trace.since(boundary) if frame["phase"] == "play"
              and frame["direction"] == "serverbound"
              and frame["packet_id"] == (0x09 if legacy else 0x11)]
    # Original click (+ the explicit no-effect resync probe on 1.16.1); never a replay.
    assert len(clicks) == (2 if legacy else 1), clicks
    assert clicks[0]["wire_sha256"] == trace.delayed_frame["wire_sha256"], clicks
    report["checks"].append(dict(name="spectator_click_no_effect", before=before, sent=sent,
                                 expired=expired, resync=resync, result=done, strict=strict,
                                 native=native, log=log, frames=clicks))

    # Control: the next ordinary click in survival is applied and Received.
    after = request("send_inventory_click", slot=9, button="left")
    control = until(status(after, "received", "diverged"), 5)
    assert control["record"]["status"] == "received" and control["effect"] is None, control
    report["checks"].append(dict(name="next_ordinary_click_received", sent=after, result=control))
    trace.expect_disconnect()
    request("disconnect")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--accept-eula", action="store_true", required=True)
    parser.add_argument("--binary", type=Path,
                        default=harness.REPO / "target/debug/examples/climbing_control_probe")
    parser.add_argument("--jars", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--version", choices=["1.16.1", "1.21.11"])
    args = parser.parse_args()
    harness.ROOT = args.output.resolve()
    for version in ([args.version] if args.version else ["1.16.1", "1.21.11"]):
        harness.run(version, args.binary.resolve(), args.jars, check=check,
                    trace_factory=DelayedClickTrace)
    reports = list(harness.ROOT.glob("*/report.json"))
    if not reports or not all(json.loads(path.read_text())["passed"] for path in reports):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
