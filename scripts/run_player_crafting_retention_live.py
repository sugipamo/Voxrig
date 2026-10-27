#!/usr/bin/env python3
"""Check received player crafting across foreign screens on official 1.16.1."""
import argparse
import json
import sys
from pathlib import Path

sys.dont_write_bytecode = True
import run_climbing_control as harness
from run_common_native import until


def check(version, command, request, trace, report):
    assert version == "1.16.1"
    command("tp ClimbingProbe 0.5 65 0.5 0 0")
    request("prepare", position=[0.5, 65.0, 0.5])
    command("clear ClimbingProbe")

    def item(slot, name, count):
        return (slot and slot["value"]["kind"] == "item"
                and slot["value"]["item"]["name"] == "minecraft:" + name
                and slot["value"]["item"]["count"] == count)

    def player_slot(index):
        return request("player")["inventory"]["slots"][index]

    def open_screen(block):
        command("setblock 0 65 2 minecraft:" + block)
        previous = (request("screen").get("screen") or {}).get("id")
        request("use_on_block", support=[0, 65, 2], face="North", cursor=[0.5, 0.5, 0.0])
        until(lambda: (request("screen").get("screen") or {}).get("id") not in (None, previous), 5)
        until(lambda: request("screen")["screen"]["full_contents_sequence"] is not None, 5)
        until(lambda: request("player")["inventory"]["cursor"] is not None, 5)

    def returned():
        grid = request("crafting")
        if not (grid and grid["source"]["kind"] == "player"
                and all(grid["inputs"]) and grid["result"]):
            return None
        record = request("inventory_send_record")["record"]
        return grid if not record or record["status"] in ("received", "diverged") else None

    def settle(sent):
        def complete():
            record = request("crafting_take_record")
            assert record["id"] == sent["id"], record
            assert record["stage"] != "requires_inspection", record["requires_inspection"]
            return record if record["stage"] != "pending" else None
        return until(complete, 5)

    # Fully received empty grids need no probe: keep their original receipts.
    initial = request("crafting")
    assert initial and all(initial["inputs"]), initial
    for block in ("chest", "crafting_table"):
        before = request("crafting")
        boundary = trace.mark()
        open_screen(block)
        request("close_screen")
        after = until(returned, 5)
        assert after["inputs"] == before["inputs"], (before, after)
        frames = trace.since(boundary)
        assert not any(f["direction"] == "clientbound" and f["phase"] == "play"
                       and f["packet_id"] == 0x14 and f.get("body_hex", "").startswith("00")
                       for f in frames), frames
        report["checks"].append(dict(name=block + "_empty_retention", before=before, after=after, frames=frames))

        # Two planks -> four sticks, admitted from received_crafting after the close.
        command("replaceitem entity ClimbingProbe inventory.0 minecraft:oak_planks 2")
        until(lambda: item(player_slot(9), "oak_planks", 2), 5)
        request("inventory_click", slot=9)
        for slot in (1, 3):
            request("inventory_click", slot=slot, button="right")
        until(lambda: item(request("crafting")["result"], "stick", 4), 5)
        filled = request("crafting")
        assert all(filled["inputs"]), filled
        sent = request("take_crafting_result")
        complete = settle(sent)
        until(lambda: item(request("player")["inventory"]["cursor"], "stick", 4), 5)
        request("inventory_click", slot=10)
        native = command("data get entity ClimbingProbe Inventory[{Slot:10b}]")
        assert 'minecraft:stick' in native and 'Count: 4b' in native, native
        report["checks"].append(dict(name=block + "_sticks_after_close", filled=filled, sent=sent,
                                    complete=complete, native=native))
        command("clear ClimbingProbe")

    # A nonempty hidden player grid must neither disappear nor acquire table inputs.
    command("replaceitem entity ClimbingProbe inventory.0 minecraft:oak_planks 1")
    until(lambda: item(player_slot(9), "oak_planks", 1), 5)
    request("inventory_click", slot=9)
    request("inventory_click", slot=1)
    until(lambda: item(request("crafting")["result"], "oak_button", 1), 5)
    before = request("crafting")
    boundary = trace.mark()
    open_screen("crafting_table")
    command("replaceitem entity ClimbingProbe inventory.0 minecraft:birch_planks 1")
    until(lambda: item(player_slot(9), "birch_planks", 1), 5)
    request("inventory_click", slot=10, container=True)
    request("inventory_click", slot=1, container=True)
    until(lambda: item(request("crafting")["result"], "birch_button", 1), 5)
    table = request("crafting")
    request("close_screen")
    # Closing the table returns its input, possibly into the selected hotbar.
    # Observe that original return before starting the independent player take.
    until(lambda: any(item(s, "birch_planks", 1)
                      for s in request("player")["inventory"]["slots"][9:45]), 5)
    after = until(returned, 5)
    # An original SET_SLOT from the preceding click may arrive after its full
    # resync. Equal values with a newer real receipt must remain authoritative.
    assert all(new and new["value"] == old["value"]
               and new["receive_sequence"] >= old["receive_sequence"]
               for old, new in zip(before["inputs"], after["inputs"])), "hidden grid changed"
    assert after["result"]["value"] == before["result"]["value"], "hidden result changed"
    complete = settle(request("take_crafting_result"))
    request("inventory_click", slot=11)
    native = command("data get entity ClimbingProbe Inventory")
    assert "minecraft:oak_button" in native and "minecraft:birch_planks" in native, native
    report["checks"].append(dict(name="nonempty_player_grid_and_foreign_changes", before=before,
                                table=table, after=after, complete=complete, native=native,
                                frames=trace.since(boundary)))

    # Repeat across vanilla tick boundaries: last-slot broadcasts and explicit
    # full refreshes do not necessarily observe the same intermediate input.
    for clear_cycle in range(12):
        # /clear also targets hidden inputs; automatic full resync must recover
        # even when no input Set Slot follows close. No explicit repair is sent.
        command("replaceitem entity ClimbingProbe inventory.0 minecraft:oak_planks 1")
        until(lambda: item(player_slot(9), "oak_planks", 1), 5)
        request("inventory_click", slot=9)
        request("inventory_click", slot=1)
        before = request("crafting")
        open_screen("chest")
        boundary = trace.mark()
        removed = command("clear ClimbingProbe")
        assert "Removed" in removed, removed
        hidden = request("player")
        request("close_screen")
        def cleared_grid():
            grid = returned()
            return grid if (grid and all(s and s["value"]["kind"] == "empty" for s in grid["inputs"])
                            and grid["result"] and grid["result"]["value"]["kind"] == "empty") else None
        try:
            after = until(cleared_grid, 5)
        except TimeoutError:
            stale = request("crafting")
            sent = request("resync_inventory")
            until(lambda: request("inventory_send_record")["record"]["status"] in ("received", "diverged"), 5)
            refreshed = request("crafting")
            assert all(s and s["value"]["kind"] == "empty" for s in refreshed["inputs"])
            report["checks"].append(dict(name="server_clear_hidden_player_grid", passed=False,
                                        cycle=clear_cycle, before=before, removed=removed, while_foreign=hidden,
                                        stale=stale, resync=sent, refreshed=refreshed,
                                        native=command("data get entity ClimbingProbe Inventory"),
                                        frames=trace.since(boundary)))
            raise AssertionError("retained player input stayed nonempty after server /clear; explicit resync supplies empty inputs") from None
        assert after["inputs"][0]["receive_sequence"] > before["inputs"][0]["receive_sequence"]
        frames = trace.since(boundary)
        assert any(f["direction"] == "serverbound" and f["phase"] == "play"
                   and f["packet_id"] == 0x09 and f.get("body_hex", "").startswith("00ffff")
                   for f in frames), "missing automatic window 0 resync"
        assert any(f["direction"] == "clientbound" and f["phase"] == "play"
                   and f["packet_id"] == 0x14 and f.get("body_hex", "").startswith("00")
                   for f in frames), "missing real window 0 full contents"
        report["checks"].append(dict(name="server_clear_hidden_player_grid", before=before,
                                    cycle=clear_cycle, removed=removed, while_foreign=hidden, after=after,
                                    frames=trace.since(boundary)))

    # Death/respawn and dimension changes must not reuse a stale nonempty grid.
    command("replaceitem entity ClimbingProbe inventory.0 minecraft:oak_planks 1")
    until(lambda: item(player_slot(9), "oak_planks", 1), 5)
    request("inventory_click", slot=9)
    request("inventory_click", slot=1)
    before = request("crafting")
    boundary = trace.mark()
    command("kill ClimbingProbe")
    until(lambda: request("player")["health"]["value"]["health"] == 0, 5)
    request("respawn")
    def respawned():
        grid = request("crafting")
        return grid if (grid and grid["session"]["world_generation"] != before["session"]["world_generation"]
                        and all(s and s["value"]["kind"] == "empty" for s in grid["inputs"])) else None
    reset = until(respawned, 15)
    report["checks"].append(dict(name="death_respawn_clears_old_grid", before=before,
                                after=reset, frames=trace.since(boundary)))
    command("replaceitem entity ClimbingProbe inventory.0 minecraft:oak_planks 1")
    until(lambda: item(player_slot(9), "oak_planks", 1), 5)
    request("inventory_click", slot=9)
    request("inventory_click", slot=1)
    before = request("crafting")
    assert item(before["inputs"][0], "oak_planks", 1)
    boundary = trace.mark()
    command("execute in minecraft:the_nether run tp ClimbingProbe 0.5 80 0.5")
    after = until(respawned, 30)
    assert after["inputs"][0]["receive_sequence"] > before["receive_sequence"]
    native = command("data get entity ClimbingProbe Inventory")
    report["checks"].append(dict(name="dimension_full_contents_replace_cleared_grid", before=before,
                                after=after, native=native, frames=trace.since(boundary)))
    trace.expect_disconnect()
    request("disconnect")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--accept-eula", action="store_true", required=True)
    parser.add_argument("--binary", type=Path, default=harness.REPO / "target/debug/examples/climbing_control_probe")
    parser.add_argument("--jars", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    harness.ROOT = args.output.resolve()
    harness.run("1.16.1", args.binary.resolve(), args.jars.resolve() if args.jars else None, check=check)
    reports = list(harness.ROOT.glob("*/report.json"))
    if not reports or not all(json.loads(path.read_text())["passed"] for path in reports):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
