#!/usr/bin/env python3
"""Original vanilla 1.16.1 craft-setup/deposit clicks, delayed cursor and refusal.

The proxy retains original frames without synthesizing or rewriting packets.
"""
import argparse
import json
from pathlib import Path
import struct
import threading
import time
import sys

sys.dont_write_bytecode = True

import run_climbing_control as harness
from run_common_native import until
from run_inventory_click_receipts import DelayedClickTrace


class CursorBoundaryProxy(DelayedClickTrace):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self.cursor_lock = threading.Lock()
        self.cursor_hold = False
        self.cursor_frames = []

    def forward_frame(self, record, destination, wire):
        body = bytes.fromhex(record.get("body_hex", ""))
        if (record["phase"] == "play" and record["direction"] == "clientbound"
                and record["packet_id"] == 0x16 and body[:3] == b"\xff\xff\xff"):
            with self.cursor_lock:
                if self.cursor_hold:
                    self.cursor_frames.append((dict(record), destination, bytes(wire)))
                    return
        super().forward_frame(record, destination, wire)

    def release_cursors(self):
        with self.cursor_lock:
            held = [r for r, _, _ in self.cursor_frames]
            for _, destination, wire in self.cursor_frames:
                destination.sendall(wire)
            self.cursor_frames.clear()
            self.cursor_hold = False
            return held


def check(version, command, request, trace, report):
    command("tp ClimbingProbe 0.5 65 0.5 0 0")
    request("prepare", position=[0.5, 65.0, 0.5])
    command("clear ClimbingProbe")

    def terminal():
        state = request("inventory_send_record")
        return state if state["record"]["status"] in ("received", "diverged", "resync_required") else None

    def item(slot, name, count):
        return (slot and slot["value"]["kind"] == "item" and
                slot["value"]["item"]["name"] == "minecraft:" + name and
                slot["value"]["item"]["count"] == count)

    def click(slot, button="left", delay=False):
        mark = trace.mark()
        if delay:
            with trace.cursor_lock:
                trace.cursor_hold = True
        sent = request("send_inventory_click", slot=slot, button=button)
        held = []
        if delay:
            until(lambda: bool(trace.cursor_frames), 3)
            incomplete = request("inventory_send_record")
            assert incomplete["record"]["status"] == "pending", incomplete
            assert incomplete["divergence"] is None, incomplete
            time.sleep(0.08)
            held = trace.release_cursors()
        done = until(terminal, 3)
        assert done["record"]["id"] == sent["id"], done
        assert done["record"]["status"] == "received" and done["divergence"] is None, done
        r = done["record"]
        assert r["after_sequence"] < r["response_sequence"] < r["contents_sequence"] < r["cursor_sequence"], r
        frames = trace.since(mark)
        clicks = [f for f in frames if f["phase"] == "play" and f["direction"] == "serverbound" and f["packet_id"] == 0x09]
        assert len(clicks) == 1 and struct.unpack(">h", bytes.fromhex(clicks[0]["body_hex"])[1:3])[0] == slot, clicks
        return dict(sent=sent, complete=done, held_original_cursors=held, frames=frames)

    # Each repetition places one ingredient, returns its remainder, takes and
    # stores the output. Strict crafting/partial comparison keeps its contract.
    for cycle in range(24):
        command("clear ClimbingProbe")
        command("replaceitem entity ClimbingProbe inventory.0 minecraft:oak_log 3")
        # RCON mutations can occur while the previous mismatch's menu updates
        # are suppressed. Obtain fresh received setup state with a separate
        # no-effect probe before admitting the next four ordinary clicks.
        setup_resync = request("resync_inventory")
        setup_boundary = until(terminal, 3)
        assert setup_boundary["record"]["id"] == setup_resync["id"], setup_boundary
        assert setup_boundary["record"]["status"] == "received", setup_boundary
        def ready_cycle():
            inventory = request("player")["inventory"]
            return (item(inventory["slots"][9], "oak_log", 3)
                    and inventory["slots"][10]["value"]["kind"] == "empty"
                    and inventory["cursor"]["value"]["kind"] == "empty")
        until(ready_cycle, 5)
        delayed = cycle >= 12
        steps = [click(9, delay=delayed), click(1, "right", delayed), click(9, delay=delayed)]
        until(lambda: item(request("crafting")["result"], "oak_planks", 4), 3)
        sent = request("send_operational_crafting_result")
        def crafted():
            result = request("operational_crafting_outcome")
            assert result["stage"] != "requires_inspection", result
            return result if result["stage"] == "crafted" else None
        outcome = until(crafted, 3)
        continued = request("continue_operational_crafting")
        steps.append(click(10, delay=delayed))
        native = command("data get entity ClimbingProbe Inventory")
        assert "minecraft:oak_planks" in native and "Count: 4b" in native, native
        assert item(request("player")["inventory"]["slots"][9], "oak_log", 2)
        assert item(request("player")["inventory"]["slots"][10], "oak_planks", 4)
        report["checks"].append(dict(name="craft_cycle", cycle=cycle, delayed=delayed,
                                     steps=steps, take=sent, outcome=outcome, continuation=continued, native=native,
                                     setup_resync=setup_boundary))

    # Withhold only original cursors across both deadline phases. Contents and
    # confirmations continue to flow; no record polling drives the timers.
    for missing in (False, True):
        mark = trace.mark()
        with trace.cursor_lock:
            trace.cursor_hold = True
        sent = request("send_inventory_click", slot=9)
        until(lambda: bool(trace.cursor_frames), 3)
        time.sleep(10.3 if missing else 5.3)
        pending = request("inventory_send_record")
        record = pending["record"]
        assert record["id"] == sent["id"] and record["deadline_resync_dispatched"], pending
        assert record["deadline_resync_after_sequence"] > record["after_sequence"], record
        if missing:
            assert record["status"] == "resync_required", pending
            assert record["inspection_reason"] == "receipt_deadline_after_resync", pending
        else:
            assert record["status"] == "pending" and pending["divergence"] is None, pending
        held = trace.release_cursors()
        if missing:
            time.sleep(0.1)
            late = request("inventory_send_record")
            assert late["record"]["status"] == "resync_required" and late["divergence"] is None, late
        else:
            late = until(terminal, 3)
            assert late["record"]["status"] == "received" and late["divergence"] is None, late
        frames = trace.since(mark)
        clicks = [f for f in frames if f["phase"] == "play" and f["direction"] == "serverbound" and f["packet_id"] == 0x09]
        assert [struct.unpack(">h", bytes.fromhex(f["body_hex"])[1:3])[0] for f in clicks] == [9, -1], clicks
        report["checks"].append(dict(name="missing_cursor_inspection" if missing else "cursor_recovered_by_no_effect_probe",
                                     sent=sent, before_release=pending, after_release=late, held=held, frames=frames))
        if missing:
            request("resync_inventory")
            until(lambda: request("inventory_send_record")["record"]["status"] in ("received", "diverged"), 3)
        click(9)  # distinct later intent from the now-received held cursor

    # A real vanilla refusal: an admitted creative armor removal is held until
    # Survival makes Curse of Binding's mayPickup predicate false. Values stay
    # exactly as before; native false by itself is not used to classify this.
    command("clear ClimbingProbe")
    command("gamemode creative ClimbingProbe")
    command('replaceitem entity ClimbingProbe armor.head minecraft:diamond_helmet{Enchantments:[{id:"minecraft:binding_curse",lvl:1s}]} 1')
    until(lambda: request("player")["game_mode"] == "creative" and item(request("player")["inventory"]["slots"][5], "diamond_helmet", 1), 5)
    trace.release.clear()
    trace.held.clear()
    trace.armed = True
    mark = trace.mark()
    try:
        sent = request("send_inventory_click", slot=5)
        assert trace.held.wait(2)
        command("gamemode survival ClimbingProbe")
        until(lambda: request("player")["game_mode"] == "survival", 3)
    finally:
        trace.release.set()
    refused = until(terminal, 3)
    assert refused["record"]["id"] == sent["id"], refused
    assert refused["record"]["status"] == "diverged" and refused["effect"] == "no_effect", refused
    assert not any(c["kind"] == "rejected" for c in refused["divergence"]["causes"]), refused
    assert item(request("player")["inventory"]["slots"][5], "diamond_helmet", 1)
    report["checks"].append(dict(name="vanilla_binding_refusal", sent=sent, result=refused,
                                 native=command("data get entity ClimbingProbe Inventory"), frames=trace.since(mark)))
    report["ordinary_clicks"] = 24 * 4
    report["additional_deadline_cases"] = 2
    report["ordinary_diverged"] = 0
    report["delayed_cursor_clicks"] = 12 * 4
    trace.expect_disconnect()
    request("disconnect")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--accept-eula", action="store_true", required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--jars", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    harness.ROOT = args.output.resolve()
    harness.run("1.16.1", args.binary.resolve(), args.jars.resolve(), check=check,
                trace_factory=CursorBoundaryProxy)
    reports = list(harness.ROOT.glob("*/report.json"))
    if not reports or not all(json.loads(p.read_text())["passed"] for p in reports):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
