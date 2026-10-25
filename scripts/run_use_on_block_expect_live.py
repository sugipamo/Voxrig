#!/usr/bin/env python3
"""Verify atomic End portal eye insertion on disposable official servers."""
import argparse
import hashlib
import json
import re
from pathlib import Path

import run_climbing_control as harness
from run_common_native import until


def check(version, command, request, trace, report):
    command("tp ClimbingProbe 0.5 65 0.5 0 0")
    request("prepare", position=[0.5, 65.0, 0.5])
    use_id = 0x2d if version == "1.16.1" else 0x3f
    mode = "survival"
    command(f"gamemode {mode} ClimbingProbe")
    until(lambda: request("player")["game_mode"] == mode)
    request("select_hotbar", slot=0)
    for offhand in (False, True):
        slot = 45 if offhand else 36
        command("clear ClimbingProbe")
        command("setblock 0 65 2 minecraft:end_portal_frame[eye=false,facing=north]")
        replace = "replaceitem entity" if version == "1.16.1" else "item replace entity"
        hand = "weapon.offhand" if offhand else "weapon.mainhand"
        with_word = "" if version == "1.16.1" else "with "
        command(f"{replace} ClimbingProbe {hand} {with_word}minecraft:ender_eye 3")

        def ready():
            p = request("player")
            held = p["inventory"]["slots"][slot]
            return p if held and held["value"]["kind"] == "item" and (
                held["value"]["item"]["name"] == "minecraft:ender_eye" and
                held["value"]["item"]["count"] == 3) else None

        before = until(ready, 10)
        args = dict(support=[0, 65, 2], face="Up", cursor=[0.5, 1.0, 0.5], offhand=offhand, mode=mode)
        for expected_slot, expected_item in ((1, "minecraft:ender_eye"), (0, "minecraft:apple")):
            boundary = trace.mark()
            refused = request("use_on_block_expect", slot=expected_slot, item=expected_item, **args)
            assert refused["kind"] == "State" and refused["refusal"], refused
            assert refused["refusal"]["expected_selected_hotbar"] == expected_slot, refused
            assert refused["refusal"]["observed_selected_hotbar"] == 0, refused
            request("wait", ms=150)
            frames = trace.since(boundary)
            assert not any(f["direction"] == "serverbound" and f["phase"] == "play" and f["packet_id"] == use_id for f in frames), frames
            assert "Test passed" in command("execute if block 0 65 2 minecraft:end_portal_frame[eye=false]"), refused
            report["checks"].append(dict(name="refused", mode=mode, offhand=offhand, expected_slot=expected_slot, expected_item=expected_item, result=refused, frames=frames))
        boundary = trace.mark()
        sent = request("use_on_block_expect", slot=0, item="minecraft:ender_eye", **args)
        assert "receipt" in sent, sent
        native = until(lambda: (r if "Test passed" in (r := command("execute if block 0 65 2 minecraft:end_portal_frame[eye=true]")) else None), 5)
        after = until(lambda: (p if (p := request("player"))["inventory"]["slots"][slot]["value"]["item"]["count"] == 2 else None), 5)
        native_slot = -106 if offhand else 0
        native_path = "equipment.offhand" if offhand and version == "1.21.11" else f"Inventory[{{Slot:{native_slot}b}}]"
        native_inventory = command("data get entity ClimbingProbe " + native_path)
        assert "minecraft:ender_eye" in native_inventory, native_inventory
        assert re.search(r"Count: ?2b" if version == "1.16.1" else r"count: ?2(?:[, }])", native_inventory), native_inventory
        frames = trace.since(boundary)
        uses = [f for f in frames if f["direction"] == "serverbound" and f["phase"] == "play" and f["packet_id"] == use_id]
        assert len(uses) == 1, uses
        assert uses[0]["body_hex"].startswith("01" if offhand else "00"), uses
        if version == "1.16.1":
            assert sent["receipt"]["interaction_sequence"] is None, sent
        else:
            assert sent["receipt"]["interaction_sequence"] is not None, sent
        report["checks"].append(dict(name="inserted_eye", mode=mode, offhand=offhand, before=before, sent=sent, after=after, native=native, native_inventory=native_inventory, frames=frames))
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
               harness.REPO / "examples/climbing_control_probe.rs",
               harness.REPO / "scripts/run_climbing_control.py",
               harness.REPO / "scripts/run_common_native.py", Path(__file__).resolve()]
    manifest = {str(p.relative_to(harness.REPO)): hashlib.sha256(p.read_bytes()).hexdigest()
                for p in sorted(sources)}
    (args.output / "inputs.json").write_text(json.dumps(dict(source_sha256=manifest,
        binary_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest()), indent=2) + "\n")


if __name__ == "__main__":
    main()
