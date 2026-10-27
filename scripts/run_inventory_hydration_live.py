#!/usr/bin/env python3
"""Check #424 with controlled missing client knowledge and real 1.16.1 receipts.

Build the library test executable with cargo test -j2 --lib --no-run, then pass
its path as --test-binary. Only the test driver can forget local grid knowledge;
no server packets are fabricated, reordered, or rewritten. A timing gate holds
one unchanged original click for the known-external-change counterexample.
"""
import argparse
import json
from pathlib import Path
import shlex
import subprocess
import sys

sys.dont_write_bytecode = True

import run_climbing_control as harness
from run_common_native import until
from run_inventory_click_receipts import DelayedClickTrace


def driver(binary):
    name = "versions::java_1_16_1::client::inventory_execution::hydration_live::driver"
    child = subprocess.Popen([binary, "--exact", name, "--ignored", "--nocapture"],
                             stdout=subprocess.PIPE, text=True)
    for line in child.stdout:
        # libtest also prints its own human-readable progress lines.
        start = line.find("{")
        if start >= 0:
            try:
                value = json.loads(line[start:])
            except json.JSONDecodeError:
                pass
            else:
                print(json.dumps(value), flush=True)
                continue
        print(line, end="", file=sys.stderr, flush=True)
    raise SystemExit(child.wait())


def check(version, command, request, trace, report):
    assert version == "1.16.1"
    report["fixture"] = "Test-only loss of client slots 0..4; unchanged official server packets"

    def terminal(sent):
        result = request("inventory_send_record")
        assert result["record"]["id"] == sent["id"], result
        return result if result["record"]["status"] in ("received", "diverged") else None

    def item(player, slot, name):
        value = player["inventory"]["slots"][slot]
        return (value and value["value"]["kind"] == "item"
                and value["value"]["item"]["name"] == "minecraft:" + name)

    for destination in [36, 37]:
        for external in [False, True]:
            command("clear ClimbingProbe")
            command("replaceitem entity ClimbingProbe inventory.0 minecraft:wooden_pickaxe 1")
            until(lambda: item(request("player"), 9, "wooden_pickaxe"), 5)
            request("forget_grid")
            before = request("player")
            assert before["inventory"]["slots"][:5] == [None] * 5, before
            assert before["inventory"]["cursor"]["value"]["kind"] == "empty", before
            assert before["inventory"]["slots"][19]["value"]["kind"] == "empty", before
            boundary = trace.mark()
            trace.held.clear()
            trace.release.clear()
            trace.armed = external
            try:
                sent = request("send_inventory_click", slot=9)
                if external:
                    assert trace.held.wait(2), "original click was not held"
                    command("replaceitem entity ClimbingProbe inventory.10 minecraft:dirt 1")
                    until(lambda: item(request("player"), 19, "dirt"), 5)
                    assert request("inventory_send_record")["record"]["status"] == "pending"
            finally:
                trace.release.set()
            done = until(lambda: terminal(sent), 5)
            strict = request("inventory_record")
            after = request("player")
            assert strict["stage"] == "observed_clicked", strict
            assert strict["legacy_reply"]["accepted"] is False, strict
            assert all(v and v["value"]["kind"] == "empty"
                       and v["source"]["sequence"] > sent["after_sequence"]
                       for v in after["inventory"]["slots"][:5]), after
            assert after["inventory"]["slots"][9]["value"]["kind"] == "empty", after
            assert after["inventory"]["cursor"]["value"]["item"]["name"] == "minecraft:wooden_pickaxe", after
            assert done["record"]["status"] == ("diverged" if external else "received"), done
            if external:
                divergence = done["divergence"]
                assert divergence["before"]["player_slots"][:5] == [None] * 5, divergence
                assert not divergence["actual"]["complete"], divergence
                assert {"kind": "external"} in divergence["causes"], divergence
                assert {"kind": "rejected"} in divergence["causes"], divergence
            else:
                assert done["divergence"] is None, done
            native_pickup = command("data get entity ClimbingProbe Inventory")
            assert "minecraft:wooden_pickaxe" not in native_pickup, native_pickup
            # Derive the remaining swap from the received cursor, never replay 9.
            placed = request("send_inventory_click", slot=destination)
            placed_result = until(lambda: terminal(placed), 5)
            assert placed_result["record"]["status"] == "received", placed_result
            final = request("player")
            assert item(final, destination, "wooden_pickaxe"), final
            assert final["inventory"]["cursor"]["value"]["kind"] == "empty", final
            native_placed = command("data get entity ClimbingProbe Inventory")
            assert "minecraft:wooden_pickaxe" in native_placed, native_placed
            clicks = [f for f in trace.since(boundary) if f["phase"] == "play"
                      and f["direction"] == "serverbound" and f["packet_id"] == 0x09]
            assert len(clicks) == 2, clicks
            if external:
                assert clicks[0]["wire_sha256"] == trace.delayed_frame["wire_sha256"]
            report["checks"].append(dict(destination=destination, external=external,
                before=before, sent=sent, result=done, strict=strict, after=after,
                placed=placed, placed_result=placed_result, final=final,
                native_pickup=native_pickup, native_placed=native_placed,
                frames=trace.since(boundary)))
    trace.expect_disconnect()
    request("disconnect")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--driver", type=Path, help=argparse.SUPPRESS)
    parser.add_argument("--accept-eula", action="store_true")
    parser.add_argument("--test-binary", type=Path)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.driver:
        driver(str(args.driver.resolve()))
    if not args.accept_eula or not args.test_binary or not args.output:
        parser.error("--accept-eula, --test-binary, and --output are required")
    harness.ROOT = args.output.resolve()
    harness.ROOT.mkdir(parents=True, exist_ok=True)
    shim = harness.ROOT / "driver.sh"
    shim.write_text("#!/bin/sh\nexec " + shlex.join([
        sys.executable, str(Path(__file__).resolve()), "--driver",
        str(args.test_binary.resolve())]) + "\n")
    shim.chmod(0o700)
    harness.run("1.16.1", shim, None, check=check, trace_factory=DelayedClickTrace)
    reports = list(harness.ROOT.glob("*/report.json"))
    if not reports or not all(json.loads(p.read_text())["passed"] for p in reports):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
