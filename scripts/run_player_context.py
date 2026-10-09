#!/usr/bin/env python3
"""Common context receipts against original packets and saved vanilla state."""
import argparse
import functools
import gzip
import hashlib
from pathlib import Path
import re
import struct
import subprocess

from run_climbing_control import REPO, run
from run_common_native import NativeSocialReader, until


def saved_weather(path):
    """Read the isolated server's actual level.dat; retain weather fields only."""
    raw = gzip.decompress(path.read_bytes())
    if len(raw) > 4 * 1024 * 1024:
        raise RuntimeError("saved level exceeds fixture budget")
    cursor = 0

    def take(n):
        nonlocal cursor
        if n < 0 or cursor + n > len(raw):
            raise RuntimeError("truncated saved NBT")
        value = raw[cursor:cursor + n]
        cursor += n
        return value

    def string():
        return take(int.from_bytes(take(2), "big")).decode()

    def value(tag, depth=0):
        if depth > 64:
            raise RuntimeError("saved NBT depth exceeds fixture budget")
        if tag in (1, 2, 3, 4, 5, 6):
            return struct.unpack({1: ">b", 2: ">h", 3: ">i", 4: ">q", 5: ">f", 6: ">d"}[tag],
                                 take({1: 1, 2: 2, 3: 4, 4: 8, 5: 4, 6: 8}[tag]))[0]
        if tag == 8:
            return string()
        if tag == 10:
            result = {}
            while child := take(1)[0]:
                name = string()
                result[name] = value(child, depth + 1)
            return result
        if tag in (7, 9, 11, 12):
            child = take(1)[0] if tag == 9 else {7: 1, 11: 3, 12: 4}[tag]
            count = int.from_bytes(take(4), "big", signed=True)
            if not 0 <= count <= 262144:
                raise RuntimeError("saved NBT array exceeds fixture budget")
            return [value(child, depth + 1) for _ in range(count)]
        raise RuntimeError("unsupported saved NBT tag")

    tag = take(1)[0]
    string()
    data = value(tag)["Data"]
    if cursor != len(raw):
        raise RuntimeError("trailing saved level NBT")
    return {key: data[key] for key in ("raining", "thundering", "rainTime", "thunderTime")}


def unpack_position(raw):
    packed = int.from_bytes(raw, "big")
    parts = [packed >> 38, packed & 4095, (packed >> 12) & 67108863]
    return [n - (1 << bits) if n >= (1 << (bits - 1)) else n for n, bits in zip(parts, (26, 12, 26))]


def verify_sources(version, context, trace):
    peers = [p for p in trace.since(0) if p["direction"] == "clientbound"
             and p["phase"] in ("configuration", "play")]
    fields = [("experience", context["experience"]), ("default_spawn", context["default_spawn"])]
    fields += [(key, receipt) for key, receipt in context["weather"].items()]
    fields += [(key, receipt) for key, receipt in context["world_view"].items()]
    verified = []
    for name, receipt in fields:
        if receipt is None:
            continue
        source = receipt["source"]
        if source["kind"] != "received" or not context["session"]["world_generation"] <= source["sequence"] <= context["receive_sequence"]:
            raise RuntimeError("context field crossed its original receive/world boundary")
        packet = peers[source["sequence"] - 1]
        r = NativeSocialReader(packet, version)
        old = version == "1.16.1"
        expected_id = {"experience": 0x48 if old else 0x65, "default_spawn": 0x42 if old else 0x5f,
                       "raining": 0x1e if old else 0x26, "rain_level": 0x1e if old else 0x26,
                       "thunder_level": 0x1e if old else 0x26, "center": 0x40 if old else 0x5c,
                       "distance": 0x41 if old else 0x5d, "simulation_distance": 0x6d}[name]
        if packet["phase"] != "play" or packet["packet_id"] != expected_id:
            raise RuntimeError("context source points to another original packet")
        actual = receipt["value"]
        if name == "experience":
            fraction = r.take(4)
            decoded = dict(level=r.integer(), total=r.integer())
            if fraction != struct.pack(">f", actual["progress"]) or decoded != {k: actual[k] for k in decoded}:
                raise RuntimeError("experience differs from its original fields")
        elif name in ("raining", "rain_level", "thunder_level"):
            reason, fraction = r.take(1)[0], r.take(4)
            if name == "raining":
                if reason not in (1, 2) or actual != (reason == 1):
                    raise RuntimeError("weather start/stop differs from original event")
            elif reason != {"rain_level": 7, "thunder_level": 8}[name] or fraction != struct.pack(">f", actual):
                raise RuntimeError("weather intensity differs from original float")
        elif name == "default_spawn":
            dimension = None if old else r.string()
            position = unpack_position(r.take(8))
            yaw, pitch = (None, None) if old else struct.unpack(">ff", r.take(8))
            if position != actual["position"] or dimension != actual["dimension"]:
                raise RuntimeError("spawn differs from original global position")
            for key, native in (("yaw", yaw), ("pitch", pitch)):
                if native is None and actual[key] is not None or native is not None and struct.pack(">f", native) != struct.pack(">f", actual[key]):
                    raise RuntimeError("spawn orientation differs from its actual version fields")
        elif name == "center":
            if actual != [r.integer(), r.integer()]:
                raise RuntimeError("view center differs from original fields")
        elif actual != r.integer():
            raise RuntimeError("view distance differs from original field")
        r.end()
        verified.append(dict(field=name, receipt=receipt, original=packet))
    return verified


def check(version, command, request, trace, report, *, sdk):
    report["sdk"] = sdk

    def snapshot(name):
        context = request("player_context")
        report["checks"].append(dict(name=name, context=context,
                                     originals=verify_sources(version, context, trace)))
        return context

    def native_xp():
        result = {}
        for key, field in (("progress", "XpP"), ("level", "XpLevel"), ("total", "XpTotal")):
            text = command("data get entity ClimbingProbe " + field)
            match = re.search(r": ([+-]?[0-9.]+)(?:f)?$", text)
            if not match:
                raise RuntimeError("native experience missing: " + text)
            result[key] = float(match[1]) if key == "progress" else int(match[1])
        return result

    initial = snapshot("initial_version_fields")
    if version == "1.16.1" and initial["world_view"]["simulation_distance"] is not None:
        raise RuntimeError("legacy simulation distance was invented")
    for name, commands in (("zero", ("experience set ClimbingProbe 0 levels", "experience set ClimbingProbe 0 points")),
                           ("points", ("experience add ClimbingProbe 5 points",)),
                           ("level", ("experience set ClimbingProbe 7 levels",))):
        before = request("player_context")
        for text in commands:
            command(text)
        native = native_xp()
        def received_xp():
            context = request("player_context")
            receipt = context["experience"]
            if receipt is None:
                return None
            actual = receipt["value"]
            same = actual["level"] == native["level"] and actual["total"] == native["total"]
            same &= struct.pack(">f", actual["progress"]) == struct.pack(">f", native["progress"])
            return context if same and (name == "zero" or receipt["source"]["sequence"] > before["receive_sequence"]) else None
        until(received_xp, 5)
        context = snapshot("experience_" + name)
        report["checks"][-1]["native"] = native
        if name == "zero" and context["experience"]["value"] != dict(progress=0.0, level=0, total=0):
            raise RuntimeError("fresh known-zero XP was replaced with a missing/default field")
    original_xp = context["experience"]
    for weather in ("clear", "rain", "thunder", "clear"):
        command("weather " + weather + " 100000")
        expected = weather != "clear"
        if weather != "clear" or context["weather"]["raining"] is not None:
            until(lambda: (value if (value := request("player_context"))["weather"]["raining"] is not None
                           and value["weather"]["raining"]["value"] == expected else None), 5)
        if weather == "thunder":
            until(lambda: (value if (value := request("player_context"))["weather"]["thunder_level"] is not None
                           and value["weather"]["thunder_level"]["value"] > 0 else None), 5)
        command("save-all flush")
        native = saved_weather(Path(trace.log.name).parent / "world/level.dat")
        if bool(native["raining"]) != expected or bool(native["thundering"]) != (weather == "thunder"):
            raise RuntimeError("official saved weather differs from actual command result")
        context = snapshot("weather_" + weather)
        report["checks"][-1]["native_saved_weather"] = native
        if context["experience"] != original_xp:
            raise RuntimeError("unrelated weather packets refreshed or changed XP")
    for pos in ([3, 65, -2], [-7, 65, 9]):
        before = request("player_context")
        command("setworldspawn " + " ".join(map(str, pos)) + (" 37" if version == "1.21.11" else ""))
        until(lambda: (value if (value := request("player_context"))["default_spawn"] is not None
                       and value["default_spawn"]["value"]["position"] == pos
                       and value["default_spawn"]["source"]["sequence"] > before["receive_sequence"] else None), 5)
        snapshot("default_spawn_" + str(pos))
    command("fill 32 64 -48 64 64 -16 minecraft:stone")
    command("tp ClimbingProbe 48.5 65 -32.5 0 0")
    until(lambda: (value if (value := request("player_context"))["world_view"]["center"] is not None
                   and value["world_view"]["center"]["value"] == [3, -3] else None), 5)
    context = snapshot("explicit_view_center")
    old_generation = context["session"]["world_generation"]
    command("execute in minecraft:the_nether run tp ClimbingProbe 0.5 80 0.5 0 0")
    until(lambda: (value if (value := request("player_context"))["session"]["world_generation"] != old_generation else None), 10)
    snapshot("new_world_sources_and_absence")
    command("experience add ClimbingProbe 1 points")
    until(lambda: (value if (value := request("player_context"))["experience"] is not None else None), 5)
    context = snapshot("new_world_fresh_experience")
    report["checks"][-1]["native"] = native_xp()
    trace.expect_disconnect()
    closed = request("context_disconnect")
    if closed["session"] != context["session"] or closed["experience"] != context["experience"]:
        raise RuntimeError("closed context lost original same-world XP receipt")
    report["checks"].append(dict(name="read_after_close", context=closed,
                                 originals=verify_sources(version, closed, trace)))


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
               artifacts={p: hashlib.sha256((REPO / p).read_bytes()).hexdigest() for p in (
                   "Cargo.lock", "scripts/run_player_context.py", "scripts/run_climbing_control.py",
                   "scripts/run_common_native.py", "examples/climbing_control_probe.rs")})
    for version in args.version or ("1.16.1", "1.21.11"):
        run(version, args.binary.resolve(), args.jars.resolve() if args.jars else None,
            check=functools.partial(check, sdk=sdk))


if __name__ == "__main__":
    main()
