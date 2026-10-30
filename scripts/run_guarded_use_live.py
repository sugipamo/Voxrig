#!/usr/bin/env python3
"""Run guarded use true/false and explicit empty UseOn on a disposable official server."""
import argparse
import hashlib
import json
from pathlib import Path
import run_climbing_control as harness
from run_common_native import until


def check(version, command, request, trace, report):
    command("difficulty peaceful")
    command("gamemode survival ClimbingProbe")
    command("fill -3 64 -3 3 64 3 minecraft:stone")
    command("setblock 1 65 0 minecraft:lever[face=floor,facing=north,powered=false]")
    command("tp ClimbingProbe 0.5 65 0.5 0 0")
    request("prepare", position=[0.5, 65.0, 0.5])
    request("select_hotbar", slot=0)
    command("clear ClimbingProbe")
    until(lambda: request("player")["inventory"]["slots"][36]["value"]["kind"] == "empty", 5)
    for op, packet_id in (("use_item_guarded", 0x2e), ("use_on_block_guarded", 0x2d)):
        for allow in (False, True):
            boundary = trace.mark()
            result = request(op, slot=0, item=None, allow=allow,
                             support=[1, 65, 0], face="Up", cursor=[0.5, 0.5, 0.5])
            assert ("Sent" if allow else "NotSent") in result, result
            if not allow:
                assert result["NotSent"]["reason"] == "GuardRejected", result
            else:
                assert result["Sent"]["cut"]["held"]["value"]["kind"] == "empty", result
            if op == "use_on_block_guarded":
                powered = "true" if allow else "false"
                until(lambda: "Test passed" in command(
                    f"execute if block 1 65 0 minecraft:lever[powered={powered}]"), 5)
            request("wait", ms=200)
            frames = trace.since(boundary)
            uses = [f for f in frames if f["phase"] == "play" and
                    f["direction"] == "serverbound" and f["packet_id"] == packet_id]
            assert len(uses) == int(allow), uses
            report["checks"].append(dict(name=op, allow=allow, result=result, frames=frames))
    # Positive held item use is also exercised, without inferring server effects.
    command("replaceitem entity ClimbingProbe weapon.mainhand minecraft:stick 1")
    until(lambda: (p := request("player"))["inventory"]["slots"][36]["value"]["kind"] == "item", 5)
    for allow in (False, True):
        boundary = trace.mark()
        result = request("use_item_guarded", slot=0, item="minecraft:stick", allow=allow)
        assert ("Sent" if allow else "NotSent") in result, result
        request("wait", ms=200)
        frames = trace.since(boundary)
        uses = [f for f in frames if f["phase"] == "play" and f["direction"] == "serverbound" and f["packet_id"] == 0x2e]
        assert len(uses) == int(allow), uses
        report["checks"].append(dict(name="held_use_item", allow=allow, result=result, frames=frames))
    trace.expect_disconnect()
    request("disconnect")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--accept-eula", action="store_true", required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--jars", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    harness.ROOT = args.output.resolve()
    harness.run("1.16.1", args.binary.resolve(), args.jars.resolve(), check=check)
    sources = [*(harness.REPO / "src").rglob("*.rs"), harness.REPO / "Cargo.toml",
               harness.REPO / "Cargo.lock", harness.REPO / "examples/climbing_control_probe.rs",
               Path(__file__).resolve()]
    manifest = {str(p.relative_to(harness.REPO)): hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(sources)}
    (args.output / "inputs.json").write_text(json.dumps(dict(source_sha256=manifest,
        binary_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest()), indent=2) + "\n")


if __name__ == "__main__":
    main()
