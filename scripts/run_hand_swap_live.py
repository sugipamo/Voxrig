#!/usr/bin/env python3
"""Check owned hand swaps with original packets and independent vanilla RCON.

Only a temporary copy of the supplied 1.21.11 JAR is run. All processes and
credentials are owned and cleaned up by run_climbing_control.run.
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
    for mode in ("survival", "creative"):
        command("gamemode " + mode + " ClimbingProbe")
        until(lambda: request("player")["game_mode"] == mode, 5)
        for main, offhand in (("iron_pickaxe", "shield"), (None, "shield"), ("iron_pickaxe", None)):
            command("clear ClimbingProbe")
            for hand, item in (("mainhand", main), ("offhand", offhand)):
                if item:
                    command(f'item replace entity ClimbingProbe weapon.{hand} with '
                            f'minecraft:{item}[minecraft:damage=17,minecraft:custom_data={{hand_swap:42}}] 1')

            def is_item(slot, name):
                return slot is not None and slot["value"]["kind"] == ("item" if name else "empty") and (
                    name is None or slot["value"]["item"]["name"] == "minecraft:" + name)

            def before():
                player = request("player")
                slots = player["inventory"]["slots"]
                return player if is_item(slots[36], main) and is_item(slots[45], offhand) else None

            baseline = until(before, 5)
            boundary = trace.mark()
            sent = request("swap_hands")
            assert sent["send"]["dispatched"] and sent["stage"] == "pending"

            def completed():
                record = request("hand_swap_record")
                assert record["id"] == sent["id"]
                assert record["stage"] != "requires_inspection", record
                return record if record["stage"] == "observed_swapped" else None

            complete = until(completed, 5)
            for field, expected in (("main_receipt", "offhand_before"), ("offhand_receipt", "main_before")):
                assert complete[field]["value"] == sent[expected]["value"]
                assert complete[field]["source"]["sequence"] > sent["send"]["after_sequence"]
            native = {}
            for slot, name, path in (("main", offhand, "SelectedItem"),
                                     ("offhand", main, "equipment.offhand")):
                response = command("data get entity ClimbingProbe " + path)
                native[slot] = response
                if name:
                    assert "minecraft:" + name in response, response
                    assert "count: 1" in response, response
                    assert '"minecraft:damage": 17' in response, response
                    assert "hand_swap: 42" in response, response
                else:
                    assert "Found no elements" in response or response.endswith("{}"), response
            frames = trace.since(boundary)
            swaps = [f for f in frames if f["direction"] == "serverbound" and f["phase"] == "play"
                     and f["packet_id"] == 0x28 and f.get("body_hex", "").startswith("06")]
            assert len(swaps) == 1, swaps
            sequence, offset = trace.varint(bytes.fromhex(swaps[0]["body_hex"])[10:])
            assert sequence == sent["send"]["interaction_sequence"]
            assert offset + 10 == len(bytes.fromhex(swaps[0]["body_hex"]))
            assert not any(f["direction"] == "serverbound" and f["phase"] == "play"
                           and f["packet_id"] == 0x11 for f in frames), "hand swap sent a container click"
            report["checks"].append(dict(mode=mode, main=main, offhand=offhand,
                                         before=baseline, sent=sent, complete=complete,
                                         native=native, frames=frames))
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
    # The caller's server folder remains read-only, including unpacked libraries.
    with tempfile.TemporaryDirectory(prefix="voxrig-hand-swap-") as temporary:
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
                    # Preserve reviewable evidence, never server credentials or worlds.
                    for name in ("report.json", "packets.jsonl", "probe.log", "probe.stderr.log", "server.log"):
                        source = folder / name
                        if source.exists():
                            shutil.copyfile(source, args.output / name)
    sources = {}
    for base in ("src", "data"):
        for path in sorted((harness.REPO / base).rglob("*")):
            if path.is_file():
                sources[str(path.relative_to(harness.REPO))] = hashlib.sha256(path.read_bytes()).hexdigest()
    sources["examples/climbing_control_probe.rs"] = hashlib.sha256(
        (harness.REPO / "examples/climbing_control_probe.rs").read_bytes()).hexdigest()
    (args.output / "inputs.json").write_text(json.dumps(dict(
        sources=sources, binary_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest()), indent=2) + "\n")


if __name__ == "__main__":
    main()
