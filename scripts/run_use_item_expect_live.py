#!/usr/bin/env python3
"""Verify atomic item-use dispatch and food consumption on disposable official servers."""
import argparse
import hashlib
import json
import re
from pathlib import Path

import run_climbing_control as harness
from run_common_native import until


def check(version, command, request, trace, report):
    command("difficulty normal")
    command("gamemode survival ClimbingProbe")
    command("tp ClimbingProbe 0.5 65 0.5 0 0")
    request("prepare", position=[0.5, 65.0, 0.5])
    request("select_hotbar", slot=0)
    use_id = 0x2e if version == "1.16.1" else 0x40
    # Pinned native play protocol IDs: Java 1.16.1=0x2e, 1.21.11=0x40.
    for surface in ("use_item_expect", "body_use_item_expect"):
        for offhand in (False, True):
            slot = 45 if offhand else 36
            command("clear ClimbingProbe")
            replace = "replaceitem entity" if version == "1.16.1" else "item replace entity"
            hand = "weapon.offhand" if offhand else "weapon.mainhand"
            with_word = "" if version == "1.16.1" else "with "
            command(f"{replace} ClimbingProbe {hand} {with_word}minecraft:bread 3")
            command("effect give ClimbingProbe minecraft:hunger 30 255 true")

            def ready():
                p = request("player")
                held = p["inventory"]["slots"][slot]
                health = p.get("health")
                return p if held and held["value"]["kind"] == "item" and (
                    held["value"]["item"]["name"] == "minecraft:bread" and
                    held["value"]["item"]["count"] == 3 and health and health["value"]["food"] <= 14) else None

            baseline = until(ready, 10)
            command("effect clear ClimbingProbe minecraft:hunger")
            for expected_slot, expected_item in ((1, "minecraft:bread"), (0, "minecraft:apple")):
                boundary = trace.mark()
                refused = request(surface, slot=expected_slot, item=expected_item, offhand=offhand)
                assert refused["refusal"], refused
                assert refused["refusal"]["observed_selected_hotbar"] == 0, refused
                if surface == "use_item_expect":
                    assert refused["kind"] == "State", refused
                else:
                    assert refused["status"]["status"] == "rejected", refused
                request("wait", ms=150)
                frames = trace.since(boundary)
                assert not any(f["direction"] == "serverbound" and f["phase"] == "play" and f["packet_id"] == use_id for f in frames), frames
                report["checks"].append(dict(name="refused", surface=surface, offhand=offhand, expected_slot=expected_slot, expected_item=expected_item, result=refused, frames=frames))
            boundary = trace.mark()
            sent = request(surface, slot=0, item="minecraft:bread", offhand=offhand)
            assert not sent.get("refusal"), sent
            if surface == "use_item_expect":
                assert "receipt" in sent, sent
            else:
                assert sent["status"]["status"] == "accepted", sent

            def consumed():
                p = request("player")
                held = p["inventory"]["slots"][slot]
                return p if held and held["value"]["kind"] == "item" and held["value"]["item"]["count"] == 2 else None

            after = until(consumed, 8)
            assert after["health"]["value"]["food"] > baseline["health"]["value"]["food"], after
            native_slot = -106 if offhand else 0
            native_path = "equipment.offhand" if offhand and version == "1.21.11" else f"Inventory[{{Slot:{native_slot}b}}]"
            native = command("data get entity ClimbingProbe " + native_path)
            assert "minecraft:bread" in native, native
            assert re.search(r"Count: ?2b" if version == "1.16.1" else r"count: ?2(?:[, }])", native), native
            request("release_use_item")
            frames = trace.since(boundary)
            uses = [f for f in frames if f["direction"] == "serverbound" and f["phase"] == "play" and f["packet_id"] == use_id]
            assert len(uses) == 1, uses
            assert uses[0]["body_hex"].startswith("01" if offhand else "00"), uses
            report["checks"].append(dict(name="consumed", surface=surface, offhand=offhand, before=baseline, sent=sent, after=after, native=native, frames=frames))
    trace.expect_disconnect()
    request("disconnect")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--accept-eula", action="store_true", required=True)
    parser.add_argument("--version", choices=("1.16.1", "1.21.11"), required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--jars", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    harness.ROOT = args.output.resolve()
    harness.run(args.version, args.binary.resolve(), args.jars.resolve(), check=check)
    sources = [*(harness.REPO / "src").rglob("*.rs"),
               harness.REPO / "Cargo.toml", harness.REPO / "Cargo.lock",
               harness.REPO / "examples/climbing_control_probe.rs", Path(__file__).resolve()]
    manifest = {str(p.relative_to(harness.REPO)): hashlib.sha256(p.read_bytes()).hexdigest()
                for p in sorted(sources)}
    (args.output / "inputs.json").write_text(json.dumps(dict(source_sha256=manifest,
        binary_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest()), indent=2) + "\n")


if __name__ == "__main__":
    main()
