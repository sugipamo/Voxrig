#!/usr/bin/env python3
"""Actual common boat control through unchanged vanilla bubble columns.

Server timers, velocity packets and passenger removal are observed, never
simulated by the SDK. Original outbound bytes are compared with model frames.
"""
import argparse
import functools
import hashlib
from pathlib import Path
import re
import struct
import subprocess

from run_climbing_control import REPO, run
from run_common_native import PacketTraceProxy, until


def velocity_packet(version, frame):
    """Independent interpretation of the original native velocity wire fields."""
    raw = bytes.fromhex(frame["body_hex"])
    target, used = PacketTraceProxy.varint(raw)
    body = raw[used:]
    if version == "1.21.11" and frame["packet_id"] == 0x23:
        if len(body) != 57:
            raise RuntimeError("invalid original entity synchronization length")
        return target, list(struct.unpack(">ddd", body[24:48]))
    if version == "1.16.1":
        if len(body) != 6:
            raise RuntimeError("invalid original legacy velocity length")
        return target, [v / 8000.0 for v in struct.unpack(">hhh", body)]
    if body == b"\0":
        return target, [0.0] * 3
    if len(body) < 6:
        raise RuntimeError("truncated original packed velocity")
    packed = (int.from_bytes(body[2:6], "big") << 16) | (body[1] << 8) | body[0]
    scale = body[0] & 3
    consumed = 6
    if body[0] & 4:
        extra, used = PacketTraceProxy.varint(body[6:])
        scale |= extra << 2
        consumed += used
    if consumed != len(body):
        raise RuntimeError("trailing original packed velocity bytes")
    return target, [(min((packed >> (3 + axis * 15)) & 32767, 32766) * 2.0 / 32766.0 - 1.0) * scale
                    for axis in range(3)]


def check(version, command, request, trace, report, *, sdk):
    report["sdk"] = sdk
    command("gamerule " + ("randomTickSpeed" if version == "1.16.1" else "minecraft:random_tick_speed") + " 0")
    command("effect give ClimbingProbe minecraft:water_breathing 600 0 true")
    boat_type = "minecraft:boat" if version == "1.16.1" else "minecraft:oak_boat"
    selector = "@e[tag=BubbleMount,limit=1]"
    move_id, paddle_id = (0x16, 0x17) if version == "1.16.1" else (0x21, 0x22)
    velocity_id = 0x46 if version == "1.16.1" else 0x63

    def native(field="Pos"):
        text = command(f"data get entity {selector} {field}")
        match = re.search(r"\[([^]]+)\]", text)
        if not match:
            raise RuntimeError("missing native vector: " + text)
        return [float(v.strip().rstrip("df")) for v in match[1].split(",")]

    for kind in ("submerged_up", "submerged_down", "surface_up", "surface_down"):
        down = kind.endswith("down")
        command("kill @e[tag=BubbleMount]")
        command("fill -8 63 -8 8 82 8 minecraft:air")
        command("fill -8 62 -8 8 62 8 minecraft:stone")
        command("fill -8 63 -8 8 64 8 minecraft:water[level=0]")
        command("tp ClimbingProbe 0.5 65 -0.5 0 0")
        command("summon " + boat_type + ' 0.5 64.65 1.5 {Tags:["BubbleMount"],Invulnerable:1b}')
        request("prepare", position=[0.5, 65.0, -0.5])
        mounted = request("mount", type=boat_type)
        report["checks"].append(dict(name=kind + "_mount", received=mounted,
                                     native_mount=command("data get entity ClimbingProbe RootVehicle.Attach")))
        command("fill -8 62 -8 8 62 8 minecraft:" + ("magma_block" if down else "soul_sand"))
        top = 72 if kind.startswith("submerged") else 64
        command(f"fill -8 63 -8 8 {top} 8 minecraft:bubble_column[drag=" + str(down).lower() + "]")
        request("wait", ms=100)
        before = native()
        boundary = trace.mark()
        count = 22 if kind.startswith("submerged") else 115
        outcome = request("drive_until_interrupted", inputs=[dict(forward=0, strafe=0, jump=False)] * count)
        record = outcome["record"]
        report["checks"].append(dict(name=kind + "_drive", outcome=outcome, native_before=before,
                                     native_after=native(), native_motion=native("Motion")))
        frames = record["boat_motion"]["frames"]

        def arrived():
            packets = trace.since(boundary)
            moves = [p for p in packets if p["phase"] == "play" and p["direction"] == "serverbound" and p["packet_id"] == move_id]
            paddles = [p for p in packets if p["phase"] == "play" and p["direction"] == "serverbound" and p["packet_id"] == paddle_id]
            return (moves, paddles) if len(moves) >= len(frames) and len(paddles) >= len(frames) else None
        moves, paddles = until(arrived)
        if len(moves) != len(frames) or len(paddles) != len(frames):
            raise RuntimeError("bubble boat packet count differs from dispatch history")
        for frame, move, paddle in zip(frames, moves, paddles):
            body = struct.pack(">dddff", *frame["position"], *frame["rotation"])
            if version == "1.21.11":
                body += bytes([frame["on_ground"]])
            if move["body_hex"] != body.hex() or paddle["body_hex"] != bytes(frame["paddles"]).hex():
                raise RuntimeError("original bubble boat packet differs from predicted frame")
        packets = trace.since(boundary)
        report["checks"].append(dict(name=kind + "_wire", packets=packets))
        incoming = [p for p in trace.since(0) if p["direction"] == "clientbound"
                    and p["phase"] in ("configuration", "play")]
        originals = []
        target = record["id"]["mount"]["vehicle_native_id"]
        for update in record["boat_motion"]["velocity_updates"]:
            source = update["receipt"]["source"]
            original = incoming[source["sequence"] - 1]
            allowed = (velocity_id,) if version == "1.16.1" else (velocity_id, 0x23)
            if source["kind"] != "received" or original["phase"] != "play" or original["packet_id"] not in allowed:
                raise RuntimeError("boat velocity source points to another original packet")
            decoded_target, velocity = velocity_packet(version, original)
            if decoded_target != target or velocity != update["receipt"]["value"]:
                raise RuntimeError("boat velocity receipt differs from original native bytes")
            originals.append(dict(update=update, original=original))
        report["checks"].append(dict(name=kind + "_velocity_sources", verified=originals))
        if kind == "surface_down":
            if not outcome["error"] or record["stage"] != "requires_inspection" or not 0 < len(frames) < count:
                raise RuntimeError("server bubble ejection did not interrupt the finite owner")
            observed = request("vehicle")
            if observed["relation"]["value"]["kind"] != "unmounted":
                raise RuntimeError("bubble ejection passenger removal was not received")
            if "Test passed" not in command("execute unless entity @a[name=ClimbingProbe,nbt={RootVehicle:{}}]"):
                raise RuntimeError("native player remains mounted after bubble ejection")
            source = observed["relation"]["source"]
            incoming = [p for p in trace.since(0) if p["direction"] == "clientbound"
                        and p["phase"] in ("configuration", "play")]
            original = incoming[source["sequence"] - 1]
            if source["kind"] != "received" or original["packet_id"] != (0x4b if version == "1.16.1" else 0x69):
                raise RuntimeError("bubble ejection source is not the original passenger packet")
            request("wait", ms=150)
            mark = trace.mark()
            request("wait", ms=250)
            if any(p["phase"] == "play" and p["direction"] == "serverbound" and p["packet_id"] in (move_id, paddle_id)
                   for p in trace.since(mark)):
                raise RuntimeError("boat frames continued after actual bubble ejection")
            report["checks"].append(dict(name=kind + "_received_ejection", received=observed,
                                         original=original, retained=request("vehicle_record")))
        else:
            if outcome["error"] or record["stage"] != "submitted":
                raise RuntimeError("bubble boat finite control failed: " + str(outcome["error"]))
            def applied_position():
                position = native()
                return position if max(abs(a - b) for a, b in zip(position, frames[-1]["position"])) < .4 else None
            final_position = until(applied_position, 5)
            report["checks"].append(dict(name=kind + "_native_endpoint", position=final_position,
                                         predicted=frames[-1]["position"]))
            if kind.startswith("submerged"):
                if not any(f["velocity"][1] < -.1 if down else f["velocity"][1] > .1 for f in frames):
                    raise RuntimeError("underwater bubble did not supply native interior impulse")
            else:
                updates = record["boat_motion"]["velocity_updates"]
                if not any(u["receipt"]["value"][1] > 2.0 for u in updates):
                    raise RuntimeError("surface launch was not folded from an actual fresh velocity receipt")
                if not any(p["direction"] == "clientbound" and p["phase"] == "play" and p["packet_id"] == velocity_id for p in packets):
                    raise RuntimeError("surface launch has no original entity velocity packet")
                report["checks"].append(dict(name=kind + "_received_launch", updates=updates))
            completed = request("dismount")
            if completed["stage"] != "completed":
                raise RuntimeError("explicit post-bubble dismount did not complete")
            report["checks"].append(dict(name=kind + "_dismount", record=completed))
    trace.expect_disconnect()
    request("disconnect")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--accept-eula", action="store_true", required=True)
    parser.add_argument("--version", choices=("1.16.1", "1.21.11"), action="append")
    parser.add_argument("--binary", type=Path, default=REPO / "target/debug/examples/climbing_control_probe")
    parser.add_argument("--jars", type=Path)
    args = parser.parse_args()
    sdk = dict(source_revision=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPO, text=True).strip(),
               source_diff_sha256=hashlib.sha256(subprocess.check_output(["git", "diff", "HEAD"], cwd=REPO)).hexdigest(),
               binary_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest(),
               artifacts={str(p): hashlib.sha256((REPO / p).read_bytes()).hexdigest() for p in (
                   "Cargo.lock", "scripts/run_boat_bubbles.py", "scripts/run_climbing_control.py",
                   "scripts/run_common_native.py", "scripts/movement_oracle/boat_bubble_scenarios.py",
                   "scripts/movement_oracle/java_1_16_1/MovementOracle.java",
                   "scripts/movement_oracle/java_1_21_11/MovementOracle.java",
                   "data/client_api/boat_bubble_oracle.json.gz")})
    for version in args.version or ("1.16.1", "1.21.11"):
        run(version, args.binary.resolve(), args.jars.resolve() if args.jars else None,
            check=functools.partial(check, sdk=sdk))


if __name__ == "__main__":
    main()
