#!/usr/bin/env python3
"""Check owned selected-stack drops with original packets and independent vanilla RCON.

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
        for amount, count, remaining in (("one", 12, 11), ("one", 1, 0), ("all", 12, 0)):
            command("kill @e[type=minecraft:item]")
            command("clear ClimbingProbe")
            command('item replace entity ClimbingProbe weapon.mainhand with '
                    f'minecraft:dirt[minecraft:custom_name="drop",minecraft:custom_data={{selected_drop:42}}] {count}')

            def before():
                player = request("player")
                slot = player["inventory"]["slots"][36]
                return player if slot and slot["value"]["kind"] == "item" and (
                    slot["value"]["item"]["name"] == "minecraft:dirt" and
                    slot["value"]["item"]["count"] == count) else None

            baseline = until(before, 5)
            boundary = trace.mark()
            sent = request("drop_selected", amount=amount)
            assert sent["send"]["dispatched"] and sent["send"]["resync_dispatched"] and sent["stage"] == "pending"
            assert sent["main_before"]["value"] == baseline["inventory"]["slots"][36]["value"]
            assert sent["amount"] == amount

            def completed():
                record = request("selected_drop_record")
                assert record["id"] == sent["id"]
                assert record["stage"] != "requires_inspection", record
                return record if record["stage"] == "observed_decreased" else None

            complete = until(completed, 5)
            receipt = complete["main_receipt"]
            assert receipt["value"] == sent["expected_after"]
            assert receipt["source"]["sequence"] > sent["send"]["after_sequence"]
            native = command("data get entity ClimbingProbe SelectedItem")
            if remaining:
                assert "minecraft:dirt" in native and f"count: {remaining}" in native, native
                assert "selected_drop: 42" in native, native
            else:
                assert "Found no elements" in native or native.endswith("{}"), native
            # Independent server entity state is verification only, never API completion evidence.
            dropped = command("data get entity @e[type=minecraft:item,limit=1] Item")
            assert "minecraft:dirt" in dropped and f"count: {count - remaining}" in dropped, dropped
            assert "selected_drop: 42" in dropped, dropped
            frames = trace.since(boundary)
            drops = [f for f in frames if f["direction"] == "serverbound" and f["phase"] == "play"
                     and f["packet_id"] == 0x28 and f.get("body_hex", "")[:2] in ("03", "04")]
            assert len(drops) == 1, drops
            action = "04" if amount == "one" else "03"
            assert drops[0]["body_hex"].startswith(action + "00" * 9)
            sequence, offset = trace.varint(bytes.fromhex(drops[0]["body_hex"])[10:])
            assert sequence == sent["send"]["interaction_sequence"]
            assert offset + 10 == len(bytes.fromhex(drops[0]["body_hex"]))
            refresh = [f for f in frames if f["direction"] == "serverbound" and f["phase"] == "play"
                       and f["packet_id"] == 0x11]
            assert len(refresh) == 1 and refresh[0]["body_hex"] == "00ffffffff0fffff00000000", refresh
            assert refresh[0]["ordinal"] > drops[0]["ordinal"]
            report["checks"].append(dict(mode=mode, amount=amount, count=count, remaining=remaining,
                                         before=baseline, sent=sent, complete=complete,
                                         native_selected=native, native_dropped=dropped, frames=frames))
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
    with tempfile.TemporaryDirectory(prefix="voxrig-selected-drop-") as temporary:
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
