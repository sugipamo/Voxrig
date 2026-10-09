#!/usr/bin/env python3
"""Validate bounded common entity history on disposable official servers."""
import argparse
from pathlib import Path
from run_climbing_control import run
from run_common_native import until


def check(version, command, request, trace, report):
    def ask(action, **arguments):
        value = request(action, **arguments)
        assert value["ok"], value
        return value["result"]

    def entities():
        return ask("entities")["entities"]

    def projectile_at(x):
        return any(e["motion"]["entity"]["type_name"] == "minecraft:fireball" and
                   e["motion"]["position"] and e["motion"]["position"]["value"]["position"] == [x, 70., 3.]
                   for e in entities())

    ask("history_tail", maximum=1)
    command('summon minecraft:sheep 2.0 66.0 2.0 {Tags:["HistoryMob"],NoAI:1b,NoGravity:1b}')
    command('summon minecraft:fireball 3.0 70.0 3.0 {Tags:["HistoryProjectile"],Motion:[0d,0d,0d],NoGravity:1b}')
    until(lambda: projectile_at(3.))
    initial = entities()
    assert any(e["motion"]["entity"]["type_name"] == "minecraft:sheep" for e in initial)
    command("tp @e[tag=HistoryProjectile,limit=1] 4.0 70.0 3.0")
    until(lambda: projectile_at(4.))
    command("tp @e[tag=HistoryProjectile,limit=1] 5.0 70.0 3.0")
    until(lambda: projectile_at(5.))
    command("kill @e[tag=HistoryProjectile]")
    command("kill @e[tag=HistoryMob]")
    until(lambda: not any(e["motion"]["entity"]["type_name"] in ("minecraft:sheep", "minecraft:fireball") for e in entities()))
    page = ask("history", resume=True)
    assert page["gap"] is None and not page["has_more"]
    records = page["records"]
    spawns = [r["kind"]["Spawn"] for r in records if "Spawn" in r["kind"]]
    assert {m["entity"]["type_name"] for m in spawns} >= {"minecraft:sheep", "minecraft:fireball"}
    projectile = next(m for m in spawns if m["entity"]["type_name"] == "minecraft:fireball")
    positions = [r["kind"]["Motion"]["position"]["value"]["position"] for r in records
                 if "Motion" in r["kind"] and r["kind"]["Motion"]["entity"]["id"] == projectile["entity"]["id"]
                 and r["kind"]["Motion"]["position"]]
    assert [4., 70., 3.] in positions and [5., 70., 3.] in positions, positions
    assert any("Status" in r["kind"] for r in records)
    removed = [r["kind"]["Removed"]["entity"] for r in records if "Removed" in r["kind"]]
    assert projectile["entity"]["id"] in removed
    report["checks"].append(dict(name="historical_living_projectile_motion_status_removal", initial=initial, page=page))

    # Packet application time remains frozen on a later read; latest state cannot recover removed entities.
    repeat = ask("history")
    by_ordinal = {r["ordinal"]: r for r in repeat["records"]}
    assert all(by_ordinal[r["ordinal"]] == r for r in records)
    report["checks"].append(dict(name="frozen_samples_after_removal", records_count=len(records), same_records=True))
    ask("history_tail", maximum=1)
    before = ask("player")["session"]
    command("gamemode survival ClimbingProbe")
    command("kill ClimbingProbe")
    until(lambda: ask("player")["health"]["value"]["health"] <= 0)
    ask("respawn")
    until(lambda: ask("player")["session"]["world_generation"] != before["world_generation"])
    command("tp ClimbingProbe 0.5 65 0.5 0 0")
    ask("prepare", position=[0.5, 65., 0.5])
    world = ask("history", resume=True)
    assert any("WorldChanged" in r["kind"] for r in world["records"])
    corrections = [r for r in world["records"] if "OwnPositionCorrection" in r["kind"]]
    assert corrections
    assert all(r["session"]["connection_id"] == before["connection_id"] for r in world["records"])
    if version == "1.16.1":
        assert all(r["kind"]["OwnPositionCorrection"]["velocity"] is None for r in corrections)
    report["checks"].append(dict(name="actual_respawn_and_own_corrections", before=before, page=world))
    trace.expect_disconnect()
    ask("revoke")
    closed = ask("history")
    assert closed["records"] and closed["session"]["connection_id"] == before["connection_id"]
    report["checks"].append(dict(name="history_after_revocation", page=closed))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--accept-eula", action="store_true", required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--jars", type=Path, required=True)
    parser.add_argument("--version", choices=["1.16.1", "1.21.11"], action="append")
    args = parser.parse_args()
    for version in args.version or ["1.16.1", "1.21.11"]:
        run(version, args.binary.resolve(), args.jars.resolve(), check=check, server_properties={"view-distance": 6})


if __name__ == "__main__":
    main()
