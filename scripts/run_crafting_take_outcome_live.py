#!/usr/bin/env python3
"""Check crafting take outcomes on official 1.16.1 and 1.21.11 servers (audit C1).

A real take must still complete (`observed_taken` / `observed_transferred`)
although every 1.16.1 take receives the provoked `accepted=false` reply, and
`craft_once` must return Ok only for that completed take. Independent RCON
inventory reads verify the server effect. Vanilla re-validates the same cursor
and capacity rules that Voxrig checks before sending, so no official-server
command produces an unchanged full resync for an admitted take; the refusal
(`not_applied`) path itself is covered by unit and packet fixture tests.
"""
import argparse
import json
from pathlib import Path

import run_climbing_control as harness
from run_common_native import until


def check(version, command, request, trace, report):
    legacy = version == "1.16.1"
    command("tp ClimbingProbe 0.5 65 0.5 0 0")
    request("prepare", position=[0.5, 65.0, 0.5])

    def give(name, count):
        if legacy:
            command(f"replaceitem entity ClimbingProbe inventory.0 minecraft:{name} {count}")
        else:
            command(f"item replace entity ClimbingProbe inventory.0 with minecraft:{name} {count}")
        until(lambda: item(request("player")["inventory"]["slots"][9], name, count), 5)

    def item(slot, name, count):
        return (slot and slot["value"]["kind"] == "item"
                and slot["value"]["item"]["name"] == "minecraft:" + name
                and slot["value"]["item"]["count"] == count)

    def native():
        return command("data get entity ClimbingProbe Inventory")

    def settle(sent):
        def complete():
            record = request("crafting_take_record")
            assert record["id"] == sent["id"], record
            return record if record["stage"] != "pending" else None
        return until(complete, 5)

    # Cursor PICKUP: two planks -> four sticks.
    command("clear ClimbingProbe")
    give("oak_planks", 2)
    request("inventory_click", slot=9)
    for slot in (1, 3):
        request("inventory_click", slot=slot, button="right")
    until(lambda: item(request("crafting")["result"], "stick", 4), 5)
    boundary = trace.mark()
    sent = request("take_crafting_result")
    complete = settle(sent)
    assert complete["stage"] == "observed_taken", complete
    if legacy:
        # The provoked mismatch is never evidence of refusal.
        assert complete["legacy_reply"]["accepted"] is False, complete["legacy_reply"]
    until(lambda: item(request("player")["inventory"]["cursor"], "stick", 4), 5)
    request("inventory_click", slot=10)
    inventory = command("data get entity ClimbingProbe Inventory[{Slot:10b}]")
    assert "minecraft:stick" in inventory, inventory
    assert ("Count: 4b" if legacy else "count: 4") in inventory, inventory
    report["checks"].append(dict(name="cursor_take_completes", sent=sent, complete=complete,
                                 native=inventory, frames=trace.since(boundary)))

    # craft_once: fill one plank and shift-take the button.
    command("clear ClimbingProbe")
    give("oak_planks", 1)
    boundary = trace.mark()
    result = request("craft_once", ingredients=["oak_planks", None, None, None])
    assert result["error"] is None, result
    take = result["record"]["take"]
    assert take["stage"] == "observed_transferred", take
    assert take["inventory_output_increase"] == 1, take
    if legacy:
        assert take["legacy_reply"]["accepted"] is False, take["legacy_reply"]
    inventory = native()
    assert "minecraft:oak_button" in inventory, inventory
    report["checks"].append(dict(name="craft_once_completes", result=result, native=inventory,
                                 frames=trace.since(boundary)))
    command("clear ClimbingProbe")
    trace.expect_disconnect()
    request("disconnect")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--accept-eula", action="store_true", required=True)
    parser.add_argument("--binary", type=Path, default=harness.REPO / "target/debug/examples/climbing_control_probe")
    parser.add_argument("--jars", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--version", choices=("1.16.1", "1.21.11"), action="append")
    args = parser.parse_args()
    harness.ROOT = args.output.resolve()
    for version in args.version or ("1.16.1", "1.21.11"):
        harness.run(version, args.binary.resolve(), args.jars.resolve() if args.jars else None, check=check)
    reports = list(harness.ROOT.glob("*/report.json"))
    if not reports or not all(json.loads(path.read_text())["passed"] for path in reports):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
