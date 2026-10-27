#!/usr/bin/env python3
"""Observe BotManager source gaps on a disposable official Java 1.16.1 server."""
import argparse
import hashlib
import json
from pathlib import Path
import sys
import time

sys.dont_write_bytecode = True
import run_climbing_control as harness
from run_common_native import until


def check(version, command, request, trace, report):
    assert version == "1.16.1"
    initial = request("inspect")
    samples = list(initial["samples"])
    # Native capacity 1 intentionally exercises source pressure. Original server
    # entity packets produce bursts; the aggregate is continuously consumed.
    for index in range(128):
        command(f'summon minecraft:armor_stand {index % 8} 66 {index // 8} '
                '{NoGravity:1b,Tags:["ManagerGap","ManagerGapGroup' + str(index // 16) + '"]}')
    for _ in range(4):
        # Keep each RCON reply below its 4096-byte fragmentation boundary.
        for group in range(8):
            command(f'execute as @e[tag=ManagerGapGroup{group}] at @s run tp @s ~0.03125 ~ ~')
        time.sleep(0.1)
        samples.extend(request("inspect")["samples"])
    gaps = [s for s in samples if s["kind"] == "gap"]
    assert gaps and all(s["skipped"] > 0 and s["username"] == "ClimbingProbe"
                        and s["generation"] == initial["generation"] for s in gaps), samples
    assert not any(s["kind"] == "aggregate_gap" for s in samples), samples
    # An upper consumer sees a subsequent authoritative state after invalidation.
    command("gamemode creative ClimbingProbe")
    final = until(lambda: (lambda v: v if v["game_mode"] == 1 else None)(request("inspect")), 5)
    native = command("data get entity ClimbingProbe playerGameType")
    assert "1" in native, native
    report["checks"].append(dict(name="source_gap_without_downstream_lag",
                                 initial=initial, gaps=gaps, samples=samples,
                                 final=final, native=native))
    trace.expect_disconnect()
    request("disconnect")


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--accept-eula", action="store_true", required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--jars", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    harness.ROOT = args.output.resolve()
    harness.run("1.16.1", args.binary.resolve(), args.jars.resolve(), check=check)
    report_path = sorted(harness.ROOT.glob("*/report.json"))[-1]
    report = json.loads(report_path.read_text())
    sources = ["src/versions/java_1_16_1/client.rs", "src/versions/java_1_16_1/manager.rs",
               "examples/manager_gap_probe.rs", "scripts/run_manager_gap_live.py", "Cargo.lock"]
    report["source_sha256"] = {p: hashlib.sha256((harness.REPO / p).read_bytes()).hexdigest()
                               for p in sources}
    (harness.ROOT / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    if not report["passed"]:
        raise SystemExit(1)
