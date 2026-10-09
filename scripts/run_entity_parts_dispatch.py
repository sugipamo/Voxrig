#!/usr/bin/env python3
"""Check common multipart dispatch/refusal on verified isolated vanilla servers."""
import argparse
from pathlib import Path
import sys
import time

sys.dont_write_bytecode = True
from run_climbing_control import REPO, run
from run_common_native import PacketTraceProxy, until


def check(version, command, request, trace, report):
    selector = "@e[tag=PartDispatch,limit=1]"
    command("effect give ClimbingProbe minecraft:resistance 600 4 true")
    command("tp ClimbingProbe 0.5 65 0.5")

    def spawn():
        command('summon minecraft:ender_dragon 0.5 70 3.5 '
                '{NoAI:1b,DragonPhase:6,Rotation:[90.0f,0.0f],Tags:["PartDispatch"]}')

    def capture_phase(phase):
        def captured():
            value = request("capture")
            if len(value["parents"]) != 1:
                return None
            metadata = value["parents"][0]["received"]["metadata"]
            # Native phase index is version-specific; modern is observed only.
            index = "15" if version == "1.16.1" else "16"
            entry = metadata.get(index)
            return value if entry and entry["value"] == {"Int": phase} else None
        return until(captured)

    def set_phase(phase):
        command(f"data merge entity {selector} {{DragonPhase:{phase}}}")
        return capture_phase(phase)

    def attacks(boundary):
        return [frame for frame in trace.since(boundary)
                if frame["phase"] == "play" and frame["direction"] == "serverbound"
                and frame["packet_id"] == (0x0e if version == "1.16.1" else 0x19)]

    def attempt(name, mode, index, accepted, target=None, sneaking=False):
        boundary = trace.mark()
        result = request("attack", mode=mode, index=index, sneaking=sneaking)
        if result["ok"] != accepted:
            raise RuntimeError(f"{name}: unexpected dispatch result {result}")
        if accepted:
            frames = until(lambda: attacks(boundary) or None)
            if len(frames) != 1:
                raise RuntimeError(name + ": attack was repeated")
            raw = bytes.fromhex(frames[0]["body_hex"])
            native_id, offset = PacketTraceProxy.varint(raw)
            if native_id != target["native_id"] or raw[offset:] != bytes((1, sneaking)):
                raise RuntimeError(name + ": wrong original part attack payload")
            if result["receipt"]["connection_id"] != target["parent"]["session"]["connection_id"]:
                raise RuntimeError(name + ": foreign dispatch receipt")
        else:
            time.sleep(0.15)
            frames = attacks(boundary)
            if frames:
                raise RuntimeError(name + ": rejected target wrote an attack")
        report["checks"].append(dict(name=name, result=result, original_attacks=frames))

    spawn()
    sitting = capture_phase(6)
    if version != "1.16.1":
        if sitting["parents"][0]["parts"] != {"Err": "UnsupportedVersion"}:
            raise RuntimeError("modern multipart model must remain unsupported")
        report["checks"].append(dict(name="modern_model_unsupported", received=sitting))
    else:
        remembered = request("remember")
        targets = remembered["targets"]
        if len(targets) != 3:
            raise RuntimeError("missing sitting head and wing targets")
        parent = remembered["parents"][0]["received"]["motion"]["entity"]["id"]
        if [target["native_id"] for target in targets] != [parent["native_id"] + n for n in (1, 7, 8)]:
            raise RuntimeError("wrong native part order")
        report["checks"].append(dict(name="original_parent_and_derived_parts", received=remembered,
                                    native_parent=command(f"data get entity {selector}")))
        for mode in ("survival", "creative"):
            command("gamemode " + mode + " ClimbingProbe")
            until(lambda: request("player")["game_mode"] == mode)
            other = "creative" if mode == "survival" else "survival"
            attempt(mode + "_wrong_handle", other, 0, False)
            attempt(mode + "_head_dispatch", mode, 0, True, targets[0])
            attempt(mode + "_wing_dispatch", mode, 1, True, targets[1], sneaking=True)
        flying = set_phase(3)
        if flying["parents"][0]["parts"]["Ok"][0]["state"] != {"Unavailable": "FlyingHead"}:
            raise RuntimeError("flying head must be unavailable")
        report["checks"].append(dict(name="received_takeoff", received=flying))
        attempt("takeoff_rejects_old_head", "creative", 0, False)
        attempt("flying_wing_dispatch", "creative", 1, True, targets[1])
        dying = set_phase(9)
        if dying["parents"][0]["parts"] != {"Err": "DeadParent"}:
            raise RuntimeError("death phase must retire the model")
        report["checks"].append(dict(name="received_death", received=dying))
        attempt("death_rejects_old_wing", "creative", 1, False)
        command("kill " + selector)
        until(lambda: not request("capture")["parents"])
        attempt("removal_rejects_old_head", "creative", 0, False)
        spawn()
        replacement = capture_phase(6)
        fresh_parent = replacement["parents"][0]["received"]["motion"]["entity"]["id"]
        if fresh_parent == parent:
            raise RuntimeError("replacement must have its own received lifetime")
        attempt("replacement_keeps_old_head_retired", "creative", 0, False)
        fresh = request("remember")
        attempt("fresh_replacement_head_dispatch", "creative", 0, True, fresh["targets"][0])
        generation = fresh["session"]["world_generation"]
        command("execute in minecraft:the_end run tp ClimbingProbe 0.5 80 0.5")
        until(lambda: request("player")["session"]["world_generation"] != generation)
        attempt("world_change_rejects_old_head", "creative", 0, False)
    report["authority_limits"] = (
        "Actual common Client with original spawn/metadata/remove/respawn receipts and transparent "
        "original attack packets; no packet injection or target fabrication. Dispatch is verified "
        "separately from damage: this isolated NoAI fixture does not prove native hitbox equality, "
        "reach, damage, End battle or Golemkit migration. Modern model stays unsupported."
    )
    trace.expect_disconnect()
    request("disconnect")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--accept-eula", action="store_true", required=True)
    parser.add_argument("--version", choices=("1.16.1", "1.21.11"), action="append")
    parser.add_argument("--binary", type=Path, default=REPO / "target/debug/examples/entity_parts_dispatch_probe")
    parser.add_argument("--jars", type=Path)
    args = parser.parse_args()
    for version in args.version or ("1.16.1", "1.21.11"):
        run(version, args.binary.resolve(), args.jars.resolve() if args.jars else None, check=check)


if __name__ == "__main__":
    main()
