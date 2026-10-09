#!/usr/bin/env python3
"""Verify an 8192-event native source through real common-Client connections."""
import argparse
from collections import defaultdict
from pathlib import Path
import sys
import time

sys.dont_write_bytecode = True
from run_climbing_control import REPO, run


def check(version, command, request, trace, report):
    initial = request("inspect")
    expected = 8192 if version == "1.16.1" else None
    if initial["capacity"] != expected:
        raise RuntimeError("wrong source capacity declaration")
    report["checks"].append(dict(name="common_connection", received=initial,
                                native_position=command("data get entity ClimbingProbe Pos")))
    if version == "1.16.1":
        command("tp ClimbingProbe 0.5 65 0.5")
        time.sleep(0.2)
        request("drain")  # separate fixture preparation from the measured burst
        for index in range(128):
            x, z = -4 + (index % 16) * 0.5, 2 + (index // 16) * 0.5
            group = index // 16
            command('summon minecraft:armor_stand ' + f'{x} 67 {z}'
                    + ' {Tags:["CapacityBurst","CapacityGroup' + str(group) + '"],NoGravity:1b,Invulnerable:1b}')
        # Advance a native server tick between changes so tracker updates are
        # delivered separately, rather than collapsing same-tick commands.
        for _ in range(20):
            # Keep each RCON reply below its 4096-byte fragmentation boundary.
            for group in range(8):
                command(f"execute as @e[tag=CapacityGroup{group}] at @s run tp @s ~0.03125 ~ ~")
            time.sleep(0.075)
        time.sleep(0.2)
        result = request("drain")
        updates = [s for s in result["samples"] if s["kind"] == "update"]
        if len(updates) <= 4096 or not result["common_gap"]:
            raise RuntimeError("burst did not exceed the separate common notification log")
        histories = defaultdict(list)
        for sample in updates:
            histories[sample["id"]].append(sample)
        if len(histories) != 128:
            raise RuntimeError("missing tracked burst entities")
        for history in histories.values():
            first, last = history[0], history[-1]
            if first["uuid"] != last["uuid"] or last["position"][0] <= first["position"][0]:
                raise RuntimeError("native source did not retain distinct historical payloads")
        result["native_endpoint"] = command("data get entity @e[tag=CapacityBurst,limit=1,sort=nearest] Pos")
        report["checks"].append(dict(name="native_source_history_survives_common_overflow", received=result))
    trace.expect_disconnect()
    request("disconnect")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--accept-eula", action="store_true", required=True)
    parser.add_argument("--version", choices=("1.16.1", "1.21.11"), action="append")
    parser.add_argument("--binary", type=Path, default=REPO / "target/debug/examples/native_event_capacity_probe")
    parser.add_argument("--jars", type=Path)
    args = parser.parse_args()
    for version in args.version or ("1.16.1", "1.21.11"):
        run(version, args.binary.resolve(), args.jars.resolve() if args.jars else None, check=check)


if __name__ == "__main__":
    main()
