#!/usr/bin/env python3
"""Check I9 own-action inventory hints against both disposable official servers.

Throws a snowball and places a block from the selected hotbar slot, then issues
an RCON /give inside the same bounded window. The hand-slot consumption must be
tagged `own_action`; the /give lands in another slot and must stay `external`.
No packets are injected or rewritten; the shared harness owns all processes.
"""
import argparse
import json
import time
from pathlib import Path

import run_climbing_control as harness
from run_common_native import until

WINDOW = 3.0  # INVENTORY_OWN_ACTION_WINDOW


def check(version, command, request, trace, report):
    modern = version == "1.21.11"

    def replace(slot, name, count):
        command(("item replace entity ClimbingProbe hotbar.%d with " if modern else
                 "replaceitem entity ClimbingProbe hotbar.%d ") % slot
                + "minecraft:%s %d" % (name, count))

    def count(slot, name):
        value = request("player")["inventory"]["slots"][slot]
        return (value["value"]["item"]["count"] if value and value["value"]["kind"] == "item"
                and value["value"]["item"]["name"] == "minecraft:" + name else 0)

    def slots(change):
        return [s["location"]["slot"] for s in change["delta"]["slots"]
                if s["location"]["kind"] == "player"]

    command("tp ClimbingProbe 0.5 65 0.5 -90 10")
    request("prepare", position=[0.5, 65.0, 0.5])
    request("select_hotbar", slot=0)
    cases = [
        ("use_item", "snowball", 4, lambda: request("use_item")),
        ("use_on_block", "stone", 2, lambda: request(
            "use_on_block", support=[1, 64, 0], face="Up", cursor=[0.5, 1.0, 0.5])),
    ]
    for name, material, amount, act in cases:
        command("clear ClimbingProbe")
        command("setblock 1 65 0 minecraft:air")
        replace(0, material, amount)
        until(lambda: count(36, material) == amount, 5)
        # Let the previous case's window expire so each change has one hint.
        time.sleep(WINDOW + 0.5)
        baseline = request("inventory_changes")
        boundary = trace.mark()
        started = time.monotonic()
        sent = act()
        until(lambda: count(36, material) == amount - 1, 5)
        command("give ClimbingProbe minecraft:oak_log 5")
        until(lambda: count(37, "oak_log") == 5, 5)
        elapsed = time.monotonic() - started
        assert elapsed < WINDOW, ("fixture exceeded the own-action window", elapsed)
        log = request("inventory_changes", after=baseline["receive_sequence"])
        own = [c for c in log["changes"] if 36 in slots(c)]
        give = [c for c in log["changes"] if 37 in slots(c)]
        assert own and give, log
        kind = "use_item" if name == "use_item" else "use_on_block"
        for change in own:
            hints = [c for c in change["causes"] if c["kind"] == "own_action"]
            assert len(hints) == 1, change
            assert hints[0]["action"] == kind, change
            assert hints[0]["locations"] == [{"kind": "player", "slot": 36}], change
            assert (hints[0]["interaction_sequence"] is not None) == modern, change
            if slots(change) == [36]:
                assert {"kind": "external"} not in change["causes"], change
        for change in give:
            assert {"kind": "external"} in change["causes"], change
            assert all(37 not in [l["slot"] for l in c.get("locations", [])]
                       for c in change["causes"]), change
        native = command("data get entity ClimbingProbe Inventory")
        assert "minecraft:oak_log" in native, native
        if name == "use_on_block":
            placed = command("execute if block 1 65 0 minecraft:stone")
            assert "Test passed" in placed, placed
        else:
            placed = None
        frames = [f for f in trace.since(boundary) if f["phase"] == "play"
                  and f["direction"] == "serverbound"
                  and f["packet_id"] in ((0x40, 0x3f) if modern else (0x2e, 0x2d))]
        assert len(frames) == 1, frames
        report["checks"].append(dict(name=name, sent=sent, elapsed=elapsed, log=log,
                                     native_inventory=native, native_block=placed,
                                     frames=frames))
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
        harness.run(version, args.binary.resolve(), args.jars, check=check)
    reports = list(harness.ROOT.glob("*/report.json"))
    if not reports or not all(json.loads(p.read_text())["passed"] for p in reports):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
