#!/usr/bin/env python3
"""Check recipe-book placement outcomes on official 1.16.1 and 1.21.11 (audit C3).

Vanilla `handlePlaceRecipe` / `ServerPlaceRecipe` send nothing when they ignore
a request (spectator, other menu, unknown or locked recipe, grid cannot be
cleared, no increase). Voxrig therefore follows every request with one no-op
full-resync click; its full contents are the processed boundary. Cases, on one
connection per version:

1. locked recipe: refused before I/O (no request frame);
2. no materials: `observed_ghost`, followed by the resync click;
3. success: `observed_placed`, followed by the resync click;
4. a revocation the server applies before the request while its notice is
   still in flight: the server ignores the request and answers the resync with
   unchanged full contents. The client then receives the revocation, so the
   record ends in the terminal `requires_inspection` rather than staying
   `pending`. The proxy only delays forwarding of original clientbound frames;
   it never drops, injects or re-encodes them.

Every record must leave `pending` within the 5 s bound. Independent RCON reads
verify the server inventory. A consistent client view never makes vanilla
ignore an admitted request, so `observed_unchanged` itself is covered by unit
and packet fixture tests (it needs a plugin or protection).
"""
import argparse
import gzip
import hashlib
import json
import threading
import time
from pathlib import Path

import run_climbing_control as harness
from run_common_native import PacketTraceProxy, until

IDS = {"1.16.1": dict(request=0x19, click=0x09), "1.21.11": dict(request=0x26, click=0x11)}


class DelayingProxy(PacketTraceProxy):
    """Original clientbound frames wait while `holding` is set, in order."""

    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self.holding = threading.Event()
        self.released = threading.Event()
        self.released.set()

    def before_forward(self, record):
        if record["direction"] == "clientbound" and record["phase"] == "play" and self.holding.is_set():
            self.released.wait(10)

    def hold(self):
        self.released.clear()
        self.holding.set()

    def release(self):
        self.holding.clear()
        self.released.set()


def check(version, command, request, trace, report):
    legacy = version == "1.16.1"
    ids = IDS[version]
    command("tp ClimbingProbe 0.5 65 0.5 0 0")
    request("prepare", position=[0.5, 65.0, 0.5])

    def item(slot, name, count):
        return (slot and slot["value"]["kind"] == "item"
                and slot["value"]["item"]["name"] == "minecraft:" + name
                and slot["value"]["item"]["count"] == count)

    def give(name, count):
        if legacy:
            command(f"replaceitem entity ClimbingProbe inventory.0 minecraft:{name} {count}")
        else:
            command(f"item replace entity ClimbingProbe inventory.0 with minecraft:{name} {count}")
        until(lambda: item(request("player")["inventory"]["slots"][9], name, count), 5)

    def settle(sent):
        def done():
            record = request("recipe_placement_record")
            assert record["id"] == sent["id"], record
            return record if record["stage"] != "pending" else None
        started = time.monotonic()
        record = until(done, 8)
        return record, time.monotonic() - started

    def frames(boundary):
        sent = [f for f in trace.since(boundary) if f["direction"] == "serverbound" and f["phase"] == "play"]
        requests = [f for f in sent if f["packet_id"] == ids["request"]]
        clicks = [f for f in sent if f["packet_id"] == ids["click"]]
        return requests, clicks

    def place(name, expected):
        boundary = trace.mark()
        result = request("place_recipe", output="minecraft:oak_button")
        assert result["error"] is None, result
        sent = result["record"]
        assert sent["send"]["dispatched"] and sent["resync_dispatched"], sent
        record, elapsed = settle(sent)
        assert record["stage"] == expected, record
        requests, clicks = frames(boundary)
        assert len(requests) == 1 and len(clicks) == 1, (requests, clicks)
        assert clicks[0]["ordinal"] > requests[0]["ordinal"]
        return dict(name=name, result=result, record=record, settle_seconds=elapsed,
                    native=command("data get entity ClimbingProbe Inventory"),
                    frames=trace.since(boundary))

    command("clear ClimbingProbe")
    command("recipe give ClimbingProbe *")

    # 1. Locked: Voxrig refuses before I/O; nothing is sent.
    command("recipe take ClimbingProbe minecraft:oak_button")
    give("oak_planks", 1)
    time.sleep(0.5)
    boundary = trace.mark()
    result = request("place_recipe", output="minecraft:oak_button")
    assert result["error"] is not None and result["record"] is None, result
    requests, clicks = frames(boundary)
    assert not requests and not clicks, (requests, clicks)
    report["checks"].append(dict(name="locked_refused_before_io", result=result))
    command("recipe give ClimbingProbe minecraft:oak_button")
    time.sleep(0.5)

    # 2. No materials: ghost display, then the resync.
    command("clear ClimbingProbe")
    give("dirt", 1)
    check = place("no_materials_ghost", "observed_ghost")
    assert check["record"]["ghost"] is not None, check["record"]
    report["checks"].append(check)

    # 3. Success: one plank placed into the grid.
    give("oak_planks", 2)
    check = place("placed", "observed_placed")
    assert check["record"]["after"] is not None, check["record"]
    report["checks"].append(check)

    # 4. The server revokes the recipe before handling the request; its notice
    # reaches the client only after the request and resync were sent.
    trace.hold()
    try:
        command("recipe take ClimbingProbe minecraft:oak_button")
        time.sleep(0.3)
        boundary = trace.mark()
        result = request("place_recipe", output="minecraft:oak_button")
        time.sleep(0.5)
    finally:
        trace.release()
    assert result["error"] is None, result
    sent = result["record"]
    assert sent["resync_dispatched"], sent
    record, elapsed = settle(sent)
    assert record["stage"] == "requires_inspection", record
    assert record["requires_inspection"], record
    requests, clicks = frames(boundary)
    assert len(requests) == 1 and len(clicks) == 1, (requests, clicks)
    native = command("data get entity ClimbingProbe Inventory")
    # The ignored request moved nothing: one plank stays in the inventory.
    assert "minecraft:oak_planks" in native, native
    report["checks"].append(dict(name="revoked_in_flight_terminal", result=result, record=record,
                                 settle_seconds=elapsed, native=native, frames=trace.since(boundary)))
    trace.expect_disconnect()
    request("disconnect")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--accept-eula", action="store_true", required=True)
    parser.add_argument("--binary", type=Path, default=harness.REPO / "target/debug/examples/climbing_control_probe")
    parser.add_argument("--jars", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--version", choices=("1.16.1", "1.21.11"), action="append")
    parser.add_argument("--evidence", type=Path, help="gzip JSON evidence file")
    args = parser.parse_args()
    harness.ROOT = args.output.resolve()
    for version in args.version or ("1.16.1", "1.21.11"):
        harness.run(version, args.binary.resolve(), args.jars.resolve() if args.jars else None,
                    check=check, trace_factory=DelayingProxy)
    reports = list(harness.ROOT.glob("*/report.json"))
    passed = bool(reports) and all(json.loads(path.read_text())["passed"] for path in reports)
    if args.evidence:
        # Reviewable reports and input digests only, never credentials or worlds.
        sources = {}
        for base in ("src", "data"):
            for path in sorted((harness.REPO / base).rglob("*")):
                if path.is_file():
                    sources[str(path.relative_to(harness.REPO))] = hashlib.sha256(path.read_bytes()).hexdigest()
        for name in ("examples/climbing_control_probe.rs", "scripts/run_recipe_placement_refusal_live.py"):
            sources[name] = hashlib.sha256((harness.REPO / name).read_bytes()).hexdigest()
        evidence = dict(reports=[json.loads(path.read_text()) for path in sorted(reports)],
                        inputs=dict(sources=sources,
                                    binary_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest()))
        args.evidence.parent.mkdir(parents=True, exist_ok=True)
        with gzip.open(args.evidence, "wt") as output:
            json.dump(evidence, output, indent=1)
            output.write("\n")
    if not passed:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
