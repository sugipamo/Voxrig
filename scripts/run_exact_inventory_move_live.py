#!/usr/bin/env python3
"""Verify explicit equipment moves on a disposable official 1.21.11 server.

Run with --accept-eula --server-jar PATH --output PATH after building the probe.
Retains original packet traces, child receipts, independent RCON and input hashes.
"""
import argparse
import hashlib
import json
import shutil
import tempfile
from pathlib import Path

import run_climbing_control as harness
from run_common_native import until


def check(version, command, request, trace, report):
    assert version == "1.21.11"
    command("tp ClimbingProbe 0.5 65 0.5 0 0")
    request("prepare", position=[0.5, 65.0, 0.5])
    request("select_hotbar", slot=0)

    def is_item(value, name):
        return value and value["value"]["kind"] == "item" and value["value"]["item"]["name"] == "minecraft:" + name

    def clicks(boundary):
        return [f for f in trace.since(boundary) if f["direction"] == "serverbound"
                and f["phase"] == "play" and f["packet_id"] == 0x11]

    for mode in ("survival", "creative"):
        command("gamemode " + mode + " ClimbingProbe")
        until(lambda: request("player")["game_mode"] == mode, 5)
        for destination, target, incoming, displaced, binding in (
                (5, "armor.head", "diamond_helmet", "iron_helmet", False),
                (45, "weapon.offhand", "iron_pickaxe", "shield", False),
                (5, "armor.head", "diamond_helmet", "iron_helmet", True)):
            command("clear ClimbingProbe")
            for slot, name, bound in (("inventory.0", incoming, False), (target, displaced, binding)):
                data = 'minecraft:damage=7,minecraft:custom_data={exact_move:42}'
                if bound:
                    data += ',minecraft:enchantments={"minecraft:binding_curse":1}'
                command(f"item replace entity ClimbingProbe {slot} with minecraft:{name}[{data}] 1")

            def before():
                player = request("player")
                slots = player["inventory"]["slots"]
                return player if is_item(slots[9], incoming) and is_item(slots[destination], displaced) else None

            baseline = until(before, 5)
            boundary = trace.mark()
            outcome = request("inventory_move", **{"from": 9, "to": destination})
            parent = outcome["record"]
            rejected = binding and mode == "survival"
            assert parent["stage"] == ("stopped" if rejected else "observed_moved"), outcome
            assert bool(outcome["error"]) == rejected, outcome
            assert parent["completed_steps"] == (0 if rejected else 3), parent
            frames = clicks(boundary)
            assert len(frames) == (0 if rejected else 3), frames
            native = command("data get entity ClimbingProbe")
            assert "exact_move: 42" in native and '"minecraft:damage": 7' in native, native
            if not rejected:
                slots = outcome["player"]["inventory"]["slots"]
                assert is_item(slots[9], displaced) and is_item(slots[destination], incoming), outcome
                for child in parent["clicks"]:
                    assert child["stage"] == "observed_clicked" and child["send"]["dispatched"], child
                    for key in ("source_receipt", "cursor_receipt"):
                        assert child[key]["source"]["sequence"] > child["send"]["after_sequence"], child
                assert slots[destination]["value"] == baseline["inventory"]["slots"][9]["value"]
                assert slots[9]["value"] == baseline["inventory"]["slots"][destination]["value"]
                assert outcome["player"]["inventory"]["cursor"]["value"]["kind"] == "empty"
            equipment = command("data get entity ClimbingProbe equipment." + ("head" if destination == 5 else "offhand"))
            assert "minecraft:" + (displaced if rejected else incoming) in equipment, equipment
            inventory = command("data get entity ClimbingProbe Inventory")
            assert "minecraft:" + (incoming if rejected else displaced) in inventory, inventory
            for native_stack in (equipment, inventory):
                assert "exact_move: 42" in native_stack and '\"minecraft:damage\": 7' in native_stack, native_stack
            report["checks"].append(dict(mode=mode, destination=destination, binding=binding,
                                         baseline=baseline, outcome=outcome, equipment=equipment,
                                         inventory=inventory, frames=frames))

    command("gamemode survival ClimbingProbe")
    until(lambda: request("player")["game_mode"] == "survival", 5)
    command("clear ClimbingProbe")
    command('item replace entity ClimbingProbe inventory.0 with minecraft:diamond_boots[minecraft:damage=7,minecraft:custom_data={exact_move:43}] 1')
    until(lambda: is_item(request("player")["inventory"]["slots"][9], "diamond_boots"), 5)
    boundary = trace.mark()
    cancelled = request("inventory_move", **{"from": 9, "to": 8}, cancel_after_first=True)
    assert cancelled["record"]["stage"] == "stopped", cancelled
    assert cancelled["record"]["completed_steps"] == 1, cancelled
    assert len(cancelled["record"]["clicks"]) == 1 and len(clicks(boundary)) == 1, cancelled
    assert is_item(cancelled["player"]["inventory"]["cursor"], "diamond_boots"), cancelled
    # Explicit separate caller action after inspecting the stopped record; the
    # helper itself has sent no rollback. Placing the cursor lets RCON inspect data.
    returned = request("inventory_click", slot=9, button="left")
    native = command("data get entity ClimbingProbe Inventory")
    assert "minecraft:diamond_boots" in native and "exact_move: 43" in native, native
    assert '"minecraft:damage": 7' in native, native
    report["checks"].append(dict(name="cancelled_after_first", outcome=cancelled,
                                 frames=clicks(boundary)[:1], explicit_return=returned, native=native))
    trace.expect_disconnect()
    request("disconnect")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--accept-eula", action="store_true", required=True)
    parser.add_argument("--server-jar", type=Path, required=True)
    parser.add_argument("--binary", type=Path, default=harness.REPO / "target/debug/examples/climbing_control_probe")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="voxrig-exact-move-") as temporary:
        root = Path(temporary)
        jars = root / "jars"
        jars.mkdir()
        shutil.copyfile(args.server_jar, jars / "1.21.11-server.jar")
        harness.ROOT = root / "runs"
        try:
            harness.run("1.21.11", args.binary.resolve(), jars, check=check)
        finally:
            if harness.ROOT.exists():
                for folder in harness.ROOT.iterdir():
                    for name in ("report.json", "packets.jsonl", "probe.log", "probe.stderr.log", "server.log"):
                        source = folder / name
                        if source.exists():
                            shutil.copyfile(source, args.output / name)
    sources = {}
    for base in ("src", "data"):
        for path in sorted((harness.REPO / base).rglob("*")):
            if path.is_file():
                sources[str(path.relative_to(harness.REPO))] = hashlib.sha256(path.read_bytes()).hexdigest()
    for name in ("examples/climbing_control_probe.rs", "scripts/run_exact_inventory_move_live.py"):
        sources[name] = hashlib.sha256((harness.REPO / name).read_bytes()).hexdigest()
    (args.output / "inputs.json").write_text(json.dumps(dict(sources=sources,
        binary_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest()), indent=2) + "\n")


if __name__ == "__main__":
    main()
