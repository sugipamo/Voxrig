#!/usr/bin/env python3
"""Check post-furnace-close settlement against a disposable official 1.16.1 server."""
import argparse
import json
import sys
from pathlib import Path

sys.dont_write_bytecode = True

import run_climbing_control as harness
from run_common_native import until


def check(version, command, request, trace, report):
    assert version == "1.16.1"
    command("tp ClimbingProbe 0.5 65 0.5 0 0")
    request("prepare", position=[0.5, 65.0, 0.5])
    command("clear ClimbingProbe")
    command("setblock 0 65 2 minecraft:furnace")
    command("replaceitem entity ClimbingProbe inventory.0 minecraft:wooden_pickaxe 1")

    def terminal(sent):
        result = request("inventory_send_record")
        assert result["record"]["id"] == sent["id"], result
        return result if result["record"]["status"] in ("received", "diverged") else None

    def has_pickaxe():
        slot = request("player")["inventory"]["slots"][9]
        return (slot and slot["value"]["kind"] == "item"
                and slot["value"]["item"]["name"] == "minecraft:wooden_pickaxe")

    until(has_pickaxe, 5)
    for cycle in range(2):  # closing and reopening creates distinct opening identities
        previous = (request("screen").get("screen") or {}).get("id")
        request("use_on_block", support=[0, 65, 2], face="North", cursor=[0.5, 0.5, 0.0])
        until(lambda: request("screen").get("screen")
              and request("screen")["screen"]["id"] != previous, 5)
        until(lambda: request("screen")["screen"]["full_contents_sequence"] is not None, 5)
        until(lambda: request("player")["inventory"]["cursor"] is not None, 5)
        boundary = trace.mark()
        close = request("close_screen")
        assert close["dispatched"] and close["server_close_sequence"] is None, close
        before = request("player")
        assert before["inventory"]["player_screen"]["kind"] == "submitted_close", before
        sent = request("send_inventory_click", slot=9, button="left")
        done = until(lambda: terminal(sent), 5)
        strict = request("inventory_record")
        assert strict["stage"] == "observed_clicked", strict
        assert strict["legacy_reply"]["accepted"] is False, strict
        player = request("player")
        assert player["inventory"]["slots"][9]["value"]["kind"] == "empty", player
        assert player["inventory"]["cursor"]["value"]["item"]["name"] == "minecraft:wooden_pickaxe", player
        native = command("data get entity ClimbingProbe Inventory")
        assert "minecraft:wooden_pickaxe" not in native, native
        # Terminal state remains fixed, and the next original click is admitted.
        assert request("inventory_send_record") == done
        next_send = request("send_inventory_click", slot=9, button="left")
        next_done = until(lambda: terminal(next_send), 5)
        assert next_done["record"]["status"] == "received", next_done
        until(has_pickaxe, 5)
        native_after = command("data get entity ClimbingProbe Inventory")
        assert "minecraft:wooden_pickaxe" in native_after, native_after
        resync = request("resync_inventory")
        resynced = until(lambda: terminal(resync), 5)
        assert resynced["record"]["status"] == "received", resynced
        frames = trace.since(boundary)
        assert not any(f["direction"] == "clientbound" and f["phase"] == "play"
                       and f["packet_id"] == 0x13 for f in frames), frames
        clicks = [f for f in frames if f["direction"] == "serverbound"
                  and f["phase"] == "play" and f["packet_id"] == 0x09]
        assert len(clicks) == 3, clicks  # pickup, put back, explicit no-effect resync
        report["checks"].append(dict(cycle=cycle, close=close, before=before,
                                    sent=sent, result=done, strict=strict, player=player,
                                    native=native, next_send=next_send, next_result=next_done,
                                    native_after=native_after, resync=resync,
                                    resynced=resynced, frames=frames))
    trace.expect_disconnect()
    request("disconnect")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--accept-eula", action="store_true", required=True)
    parser.add_argument("--binary", type=Path,
                        default=harness.REPO / "target/debug/examples/climbing_control_probe")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    harness.ROOT = args.output.resolve()
    harness.run("1.16.1", args.binary.resolve(), None, check=check)
    reports = list(harness.ROOT.glob("*/report.json"))
    if not reports or not all(json.loads(path.read_text())["passed"] for path in reports):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
