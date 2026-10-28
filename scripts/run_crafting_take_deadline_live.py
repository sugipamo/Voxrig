#!/usr/bin/env python3
"""Healthy takes plus withheld original inventory receipts on vanilla 1.16.1.

The proxy queues original transaction/full-content/slot packets, while forwarding
all other original traffic. No packet values are synthesized. Default 5 s waits
are used; the no-effect deadline probe must never replay slot zero.
"""
import argparse
import json
from pathlib import Path
import struct
import sys
import threading
import time

sys.dont_write_bytecode = True

import run_climbing_control as harness
from run_common_native import PacketTraceProxy, until
import run_crafting_take_outcome_live as healthy


class HeldReceipts(PacketTraceProxy):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self.receipt_lock = threading.Lock()
        self.holding = False
        self.held = []
        self.delivered_held = []

    def forward_frame(self, record, destination, wire):
        if (record["phase"] == "play" and record["direction"] == "clientbound"
                and record["packet_id"] in (0x12, 0x14, 0x16)):
            with self.receipt_lock:
                if self.holding:
                    self.held.append((dict(record), destination, bytes(wire)))
                    return
                destination.sendall(wire)
                return
        super().forward_frame(record, destination, wire)

    def release(self):
        with self.receipt_lock:
            for record, destination, wire in self.held:
                destination.sendall(wire)
                self.delivered_held.append(record)
            self.held.clear()
            self.holding = False


def check(version, command, request, trace, report):
    # Existing healthy PICKUP and QUICK_MOVE scenarios, with RCON inventory checks.
    healthy.check(version, command, request, trace, report, disconnect=False)
    for entry in report["checks"]:
        record = entry.get("complete") or entry["result"]["record"]["take"]
        assert record["deadline_resync_dispatched"] is False, record
        assert record["inspection_reason"] is None, record

    command("replaceitem entity ClimbingProbe inventory.0 minecraft:oak_planks 2")
    until(lambda: request("player")["inventory"]["slots"][9]["value"]["kind"] == "item", 5)
    request("inventory_click", slot=9)
    for slot in (1, 3):
        request("inventory_click", slot=slot, button="right")
    until(lambda: request("crafting")["result"]["value"]["kind"] == "item", 5)
    mark = trace.mark()
    with trace.receipt_lock:
        trace.holding = True
    started = time.monotonic()
    sent = request("take_crafting_result")
    assert sent["stage"] == "pending", sent
    # No record inspection drives progress during either wait.
    request("wait", ms=11000)
    expired = request("crafting_take_record")
    elapsed = time.monotonic() - started
    assert expired["id"] == sent["id"], expired
    assert expired["stage"] == "requires_inspection", expired
    assert expired["inspection_reason"] == "receipt_deadline_after_resync", expired
    assert expired["deadline_resync_dispatched"] is True, expired
    assert expired["player_after_inspection"] is None, expired
    frames = trace.since(mark)
    clicks = [f for f in frames if f["phase"] == "play" and f["direction"] == "serverbound"
              and f["packet_id"] == 0x09]
    assert len(clicks) == 2, clicks
    original, probe = [bytes.fromhex(f["body_hex"]) for f in clicks]
    assert struct.unpack(">h", original[1:3])[0] == 0 and original[6] == 0, clicks
    assert struct.unpack(">h", probe[1:3])[0] == -1 and probe[6] == 0, clicks
    assert original[4:6] != probe[4:6], clicks
    assert expired["deadline_resync_action"] == struct.unpack(">h", probe[4:6])[0], expired
    with trace.receipt_lock:
        withheld = [f for f, _, _ in trace.held]
    assert {0x12, 0x14, 0x16} <= {f["packet_id"] for f in withheld}, withheld
    trace.release()
    def late_receipts():
        latest = request("crafting_take_record")
        if latest["player_after_inspection"] and latest["deadline_resync_reply"]:
            return latest
        return None
    late = until(late_receipts, 5)
    assert late["id"] == sent["id"] and late["stage"] == "requires_inspection", late
    assert late["inspection_reason"] == expired["inspection_reason"], late
    assert late["crafting_after_inspection"] is not None, late
    assert late["legacy_reply"] is not None, late
    assert len([f for f in trace.since(mark) if f["phase"] == "play"
                and f["direction"] == "serverbound" and f["packet_id"] == 0x09]) == 2
    report["checks"].append(dict(name="withheld_boundary_deadline_and_late_evidence", sent=sent,
                                 expired=expired, late=late, elapsed_seconds=elapsed,
                                 withheld_original_frames=withheld, frames=trace.since(mark)))
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
                trace_factory=HeldReceipts)
    reports = list(harness.ROOT.glob("*/report.json"))
    if not reports or not all(json.loads(path.read_text())["passed"] for path in reports):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
