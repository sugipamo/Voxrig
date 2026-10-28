#!/usr/bin/env python3
"""Check refused selected drops and hand swaps on official 1.21.11 (audit I6/I7).

Vanilla `handlePlayerAction` returns before DROP_ITEM / DROP_ALL_ITEMS /
SWAP_ITEM_WITH_OFFHAND while `hasClientLoaded()` is false (a dead player
waiting for respawn) or the player is a spectator, while the following resync
click still answers with full contents. With keep_inventory, a dead player keeps
both hands, so the full resync after the send still equals `before` and the
record must end as `observed_unchanged` instead of staying `pending`. The same
connection then respawns and checks the success paths (`observed_decreased`,
`observed_swapped`), which also shows that the unchanged records released the
inventory owner. Independent RCON reads verify the server state only.

Only a temporary copy of the supplied JAR is run; processes and credentials are
owned and cleaned up by run_climbing_control.run.
"""
import argparse
import gzip
import hashlib
import json
import shutil
import tempfile
from pathlib import Path

import run_climbing_control as harness
from run_common_native import until

PLAYER_ACTION, CONTAINER_CLICK = 0x28, 0x11
RESYNC = "00ffffffff0fffff00000000"


def check(version, command, request, trace, report):
    assert version == "1.21.11"
    command("gamerule minecraft:keep_inventory true")
    command("tp ClimbingProbe 0.5 65 0.5 0 0")
    request("prepare", position=[0.5, 65.0, 0.5])
    request("select_hotbar", slot=0)
    command("gamemode survival ClimbingProbe")
    until(lambda: request("player")["game_mode"] == "survival", 5)

    def item(slot, name, count):
        return (slot is not None and slot["value"]["kind"] == "item"
                and slot["value"]["item"]["name"] == "minecraft:" + name
                and slot["value"]["item"]["count"] == count)

    def hands(main, offhand):
        command("kill @e[type=minecraft:item]")
        command("clear ClimbingProbe")
        command(f"item replace entity ClimbingProbe weapon.mainhand with minecraft:{main[0]} {main[1]}")
        command(f"item replace entity ClimbingProbe weapon.offhand with minecraft:{offhand[0]} {offhand[1]}")

        def ready():
            slots = request("player")["inventory"]["slots"]
            return item(slots[36], *main) and item(slots[45], *offhand)
        until(ready, 5)

    def settle(name, sent):
        def done():
            record = request(name)
            assert record["id"] == sent["id"], record
            return record if record["stage"] != "pending" else None
        return until(done, 5)

    def frames_since(boundary, action):
        frames = trace.since(boundary)
        sent = [f for f in frames if f["direction"] == "serverbound" and f["phase"] == "play"]
        actions = [f for f in sent if f["packet_id"] == PLAYER_ACTION and f.get("body_hex", "")[:2] == action]
        clicks = [f for f in sent if f["packet_id"] == CONTAINER_CLICK]
        assert len(actions) == 1, actions
        assert len(clicks) == 1 and clicks[0]["body_hex"] == RESYNC, clicks
        assert clicks[0]["ordinal"] > actions[0]["ordinal"]
        return frames

    def drop(expected):
        boundary = trace.mark()
        sent = request("drop_selected", amount="one")
        assert sent["send"]["dispatched"] and sent["send"]["resync_dispatched"], sent
        record = settle("selected_drop_record", sent)
        assert record["stage"] == expected, record
        native = command("data get entity ClimbingProbe SelectedItem")
        return sent, record, native, frames_since(boundary, "04")

    def swap(expected):
        boundary = trace.mark()
        sent = request("swap_hands")
        assert sent["send"]["dispatched"] and sent["resync_dispatched"], sent
        record = settle("hand_swap_record", sent)
        assert record["stage"] == expected, record
        native = {path: command("data get entity ClimbingProbe " + path)
                  for path in ("SelectedItem", "equipment.offhand")}
        return sent, record, native, frames_since(boundary, "06")

    # Dead and not yet respawned: vanilla ignores both actions.
    hands(("dirt", 12), ("shield", 1))
    command("kill ClimbingProbe")
    dead = until(lambda: (lambda p: p if p["health"] and p["health"]["value"]["health"] <= 0 else None)(
        request("player")), 5)
    sent, record, native, frames = drop("observed_unchanged")
    assert record["main_receipt"] is None and record["requires_inspection"] is None, record
    assert record["unchanged_boundary"] > sent["send"]["after_sequence"], record
    assert "minecraft:dirt" in native and "count: 12" in native, native
    report["checks"].append(dict(name="dead_drop_unchanged", dead=dead, sent=sent, record=record,
                                 native_selected=native, frames=frames))

    # The unchanged drop released the inventory owner: the swap is admitted.
    sent, record, native, frames = swap("observed_unchanged")
    assert record["main_receipt"] is None and record["offhand_receipt"] is None, record
    assert record["unchanged_boundary"] > sent["send"]["after_sequence"], record
    assert "minecraft:dirt" in native["SelectedItem"], native
    assert "minecraft:shield" in native["equipment.offhand"], native
    report["checks"].append(dict(name="dead_swap_unchanged", sent=sent, record=record,
                                 native=native, frames=frames))

    respawned = request("respawn")
    until(lambda: (lambda p: p if p["health"] and p["health"]["value"]["health"] > 0 else None)(
        request("player")), 10)
    command("tp ClimbingProbe 0.5 65 0.5 0 0")
    request("prepare", position=[0.5, 65.0, 0.5])
    report["checks"].append(dict(name="respawn", record=respawned))

    # Success regression on the same connection.
    hands(("dirt", 12), ("shield", 1))
    sent, record, native, frames = drop("observed_decreased")
    assert record["main_receipt"]["value"] == sent["expected_after"], record
    assert record["unchanged_boundary"] is None, record
    assert "minecraft:dirt" in native and "count: 11" in native, native
    report["checks"].append(dict(name="alive_drop_decreased", sent=sent, record=record,
                                 native_selected=native, frames=frames))

    hands(("iron_pickaxe", 1), ("shield", 1))
    sent, record, native, frames = swap("observed_swapped")
    assert record["main_receipt"]["value"] == sent["offhand_before"]["value"], record
    assert record["offhand_receipt"]["value"] == sent["main_before"]["value"], record
    assert record["unchanged_boundary"] is None, record
    assert "minecraft:shield" in native["SelectedItem"], native
    assert "minecraft:iron_pickaxe" in native["equipment.offhand"], native
    report["checks"].append(dict(name="alive_swap_swapped", sent=sent, record=record,
                                 native=native, frames=frames))
    command("clear ClimbingProbe")
    trace.expect_disconnect()
    request("disconnect")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--accept-eula", action="store_true", required=True)
    parser.add_argument("--server-jar", type=Path, required=True)
    parser.add_argument("--binary", type=Path, default=harness.REPO / "target/debug/examples/climbing_control_probe")
    parser.add_argument("--output", type=Path, required=True, help="gzip JSON evidence file")
    args = parser.parse_args()
    evidence = {}
    # The caller's server folder remains read-only, including unpacked libraries.
    with tempfile.TemporaryDirectory(prefix="voxrig-drop-handswap-") as temporary:
        root = Path(temporary)
        jars = root / "jars"
        jars.mkdir()
        shutil.copyfile(args.server_jar, jars / "1.21.11-server.jar")
        harness.ROOT = root / "runs"
        try:
            harness.run("1.21.11", args.binary.resolve(), jars, check=check)
        finally:
            for path in harness.ROOT.glob("*/report.json"):
                # Reviewable evidence only, never server credentials or worlds.
                evidence["report"] = json.loads(path.read_text())
    sources = {}
    for base in ("src", "data"):
        for path in sorted((harness.REPO / base).rglob("*")):
            if path.is_file():
                sources[str(path.relative_to(harness.REPO))] = hashlib.sha256(path.read_bytes()).hexdigest()
    sources["examples/climbing_control_probe.rs"] = hashlib.sha256(
        (harness.REPO / "examples/climbing_control_probe.rs").read_bytes()).hexdigest()
    evidence["inputs"] = dict(sources=sources, binary_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest(),
                              server_jar_sha256=hashlib.sha256(args.server_jar.read_bytes()).hexdigest())
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with gzip.open(args.output, "wt") as output:
        json.dump(evidence, output, indent=1)
        output.write("\n")
    if not evidence.get("report", {}).get("passed"):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
