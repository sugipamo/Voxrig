#!/usr/bin/env python3
"""Qualify inventory comparison/history against both disposable official servers.

The timing gate holds one unchanged original click while the official server
collects a real item entity. It neither injects nor rewrites inventory packets.
"""
import argparse
import json
from pathlib import Path

import run_climbing_control as harness
from run_common_native import until
from run_inventory_click_receipts import DelayedClickTrace


def check(version, command, request, trace, report):
    def terminal():
        result = request("inventory_send_record")
        return result if result["record"]["status"] in ("received", "diverged") else None

    def count(name, slot=9):
        value = request("player")["inventory"]["slots"][slot]
        return (value["value"]["item"]["count"] if value and value["value"]["kind"] == "item"
                and value["value"]["item"]["name"] == "minecraft:" + name else 0)

    def net(log, name):
        return sum(delta["change"] for change in log["changes"]
                   for delta in change["delta"]["items"]
                   if delta["item"]["name"] == "minecraft:" + name)

    command("tp ClimbingProbe 0.5 65 0.5 0 0")
    request("prepare", position=[0.5, 65.0, 0.5])
    command("clear ClimbingProbe")
    command("replaceitem entity ClimbingProbe inventory.0 minecraft:cobblestone 3"
            if version == "1.16.1" else
            "item replace entity ClimbingProbe inventory.0 with minecraft:cobblestone 3")
    until(lambda: count("cobblestone") == 3, 5)
    normal = []
    for _ in range(2):  # take, then put back, each from actual received values
        sent = request("send_inventory_click", slot=9, button="left")
        done = until(terminal, 5)
        assert done["record"]["id"] == sent["id"], done
        assert done["record"]["status"] == "received" and done["divergence"] is None, done
        normal.append(done)
    report["checks"].append(dict(name="ordinary_click_received", results=normal))

    baseline = request("inventory_changes")
    command("give ClimbingProbe minecraft:dirt 2")
    def given():
        slots = request("player")["inventory"]["slots"]
        return sum(value["value"]["item"]["count"] for value in slots[9:46]
                   if value and value["value"]["kind"] == "item"
                   and value["value"]["item"]["name"] == "minecraft:dirt") == 2
    until(given, 5)
    external = request("inventory_changes", after=baseline["receive_sequence"])
    assert net(external, "dirt") == 2, external
    assert all(change["causes"] == [{"kind": "external"}] for change in external["changes"]), external
    report["checks"].append(dict(name="give_external", log=external))

    before = request("player")
    cursor = before["inventory"]["cursor"]
    assert count("cobblestone") == 3 and cursor["value"]["kind"] == "empty", before
    baseline = request("inventory_changes")
    boundary = trace.mark()
    trace.armed = True
    trace.release.clear()
    trace.held.clear()
    try:
        sent = request("send_inventory_click", slot=9, button="left")
        assert trace.held.wait(2), "original click did not reach timing gate"
        stack = ('{id:"minecraft:cobblestone",Count:1b}' if version == "1.16.1" else
                 '{id:"minecraft:cobblestone",count:1}')
        command('summon minecraft:item 0.5 65.1 0.5 {PickupDelay:0s,Item:' + stack + '}')
        until(lambda: count("cobblestone") == 4, 5)
        pending = request("inventory_send_record")
        assert pending["record"]["status"] == "pending", pending
        native = command("data get entity ClimbingProbe Inventory")
        assert "minecraft:cobblestone" in native, native
    finally:
        trace.release.set()
    done = until(terminal, 5)
    assert done["record"]["id"] == sent["id"], done
    assert done["record"]["status"] == "diverged", done
    divergence = done["divergence"]
    collected = [cause for cause in divergence["causes"] if cause["kind"] == "collected"]
    assert len(collected) == 1 and len(collected[0]["entity_ids"]) == 1, divergence
    assert divergence["before"]["receive_sequence"] == sent["after_sequence"], divergence
    assert divergence["current"]["cursor"]["value"]["item"]["count"] == 4, divergence
    assert divergence["intended"]["kind"] == "exact", divergence
    assert divergence["actual"]["items"][0]["change"] == 1, divergence
    assert divergence["actual"]["items"][0]["item"]["name"] == "minecraft:cobblestone", divergence
    log = request("inventory_changes", after=baseline["receive_sequence"])
    assert net(log, "cobblestone") == 1, log
    assert any(collected[0] in change["causes"] for change in log["changes"]), log
    frames = [frame for frame in trace.since(boundary) if frame["phase"] == "play"
              and frame["direction"] == "serverbound"
              and frame["packet_id"] == (0x09 if version == "1.16.1" else 0x11)]
    assert len(frames) == 1 and frames[0]["wire_sha256"] == trace.delayed_frame["wire_sha256"], frames
    report["checks"].append(dict(name="real_collect_during_original_click", before=before,
                                  sent=sent, pending=pending, native=native, result=done, log=log,
                                  frames=frames))
    trace.expect_disconnect()
    request("disconnect")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--accept-eula", action="store_true", required=True)
    parser.add_argument("--binary", type=Path, default=harness.REPO / "target/debug/examples/climbing_control_probe")
    parser.add_argument("--jars", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--version", choices=["1.16.1", "1.21.11"])
    args = parser.parse_args()
    harness.ROOT = args.output.resolve()
    for version in ([args.version] if args.version else ["1.16.1", "1.21.11"]):
        harness.run(version, args.binary.resolve(), args.jars, check=check,
                    trace_factory=DelayedClickTrace)
    reports = list(harness.ROOT.glob("*/report.json"))
    if not all(json.loads(path.read_text())["passed"] for path in reports):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
