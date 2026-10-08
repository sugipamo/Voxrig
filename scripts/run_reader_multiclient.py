#!/usr/bin/env python3
"""Bounded SDK-only eleven-client stress on unchanged official vanilla 1.16.1.

The own Java launcher invokes original broadcastEntityEvent six wolves x twice
per tick. This is controlled valid traffic, not autonomous wolf behavior or an
Evolto reproduction. Diagnostic retention selects KeepAlives; wire forwarding
never filters packets. Setup and observation histories remain bounded.
"""
import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import subprocess
import time
from run_climbing_control import REPO, run
from run_common_native import PacketTraceProxy, until


def make_trace(port, version, path):
    statuses = Counter()
    def retain(record):
        if record["phase"] == "play" and record["direction"] == "clientbound" and record["packet_id"] == 0x1b:
            body = bytes.fromhex(record.get("body_hex", ""))
            if len(body) == 5 and body[-1] == 8:
                statuses[record["connection"]] += 1
        return record["phase"] != "play" or (
            record["direction"] == "clientbound" and record["packet_id"] == 0x20) or (
            record["direction"] == "serverbound" and record["packet_id"] == 0x10)
    trace = PacketTraceProxy(port, version, path, record_filter=retain)
    trace.statuses = statuses
    return trace


def check_for(seconds):
    def check(version, command, request, trace, report):
        folder = Path(trace.log.name).parent
        until(lambda: "VOXRIG_NATIVE_READER_STRESS_READY" in (folder / "server.log").read_text())
        for index in range(11):
            result = request("connect", name=f"Reader{index:02d}")
            assert result == {"connected": True, "index": index}
        command("tp @a 0.5 65 0.5")
        for index in range(6):
            command(f'summon minecraft:wolf {index - 3}.5 65 2.5 '
                    '{NoAI:1b,Invulnerable:1b,PersistenceRequired:1b,Tags:["ReaderStress"]}')
        until(lambda: all(s["wolf_count"] == 6 for s in request("sample", ms=100)["samples"]))
        flag = folder / "voxrig-reader-stress-enabled"
        flag.touch()
        started = time.monotonic()
        samples = captures = 0
        maxima = [0] * 11
        initial_sequences = None
        while time.monotonic() - started < seconds:
            result = request("sample", ms=1000)["samples"]
            assert len(result) == 11 and all(s["wolf_count"] == 6 for s in result)
            if initial_sequences is None:
                initial_sequences = [s["receive_sequence"] for s in result]
            samples += 1
            for sample in result:
                captures += sample["captures"]
                maxima[sample["index"]] = max(maxima[sample["index"]], sample["max_capture_ms"])
            if samples % 30 == 0:
                progress = dict(elapsed_seconds=time.monotonic() - started, samples=samples,
                                captures=captures, max_capture_ms=max(maxima))
                (folder / "progress.json").write_text(json.dumps(progress) + "\n")
                print(json.dumps(progress), flush=True)
        elapsed = time.monotonic() - started
        flag.unlink()
        time.sleep(0.5)
        for connection in range(1, 12):
            trace.expect_disconnect(connection)
        for index in range(11):
            assert request("disconnect", index=index) == {"disconnected": True}
        until(lambda: "Test passed" in command("execute unless entity @a"))
        with trace.lock:
            frames = list(trace.frames)
            statuses = dict(trace.statuses)
            counts = [dict(connection=k[0], direction=k[1], phase=k[2], packet_id=k[3], count=v)
                      for k, v in sorted(trace.packet_counts.items())]
        challenges, replies = {}, {}
        for frame in frames:
            if frame["phase"] != "play": continue
            key = (frame["connection"], frame["body_hex"])
            target = challenges if frame["direction"] == "clientbound" else replies
            if key in target: raise RuntimeError("duplicate KeepAlive fixture identity")
            target[key] = frame["observed_monotonic_seconds"]
        if set(challenges) != set(replies): raise RuntimeError("unanswered or foreign KeepAlive")
        latencies = [(replies[key] - sent) * 1000 for key, sent in challenges.items()]
        if not latencies or min(latencies) < 0 or max(latencies) >= 15000:
            raise RuntimeError("missing or late KeepAlive response")
        for connection in range(1, 12):
            if statuses.get(connection, 0) / elapsed < 200:
                raise RuntimeError("fixture failed to establish requested high status rate")
            if not any(key[0] == connection for key in challenges):
                raise RuntimeError("connection did not receive a KeepAlive")
        report["checks"].append(dict(name="eleven_client_high_status_capture_soak", duration_seconds=elapsed,
            sample_cycles=samples, captures=captures, max_capture_ms_by_client=maxima,
            first_receive_sequences=initial_sequences, final_samples=result,
            status8_by_connection=statuses, status8_per_second_by_connection={k:v/elapsed for k,v in statuses.items()},
            keepalive_challenges=len(challenges), keepalive_replies=len(replies),
            max_proxy_observed_response_ms=max(latencies), packet_counts=counts,
            diagnostic_retention="KeepAlive and non-play only; original wire bytes all forwarded",
            scope="SDK common API only, four Tokio workers, six forced native status-8 wolves; no Evolto or Golemkit consumer"))
        request("quit")
    return check


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--accept-eula", action="store_true", required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--jars", type=Path, required=True)
    parser.add_argument("--seconds", type=int, default=1800)
    args = parser.parse_args()
    if not 30 <= args.seconds <= 14400: parser.error("seconds must be 30..14400")
    classes = REPO / ".local/climbing/reader-stress-classes"
    classes.mkdir(parents=True, exist_ok=True)
    source = REPO / "scripts/NativeReaderStressControl.java"
    subprocess.run(["javac", "-d", str(classes), str(source)], check=True)
    run("1.16.1", args.binary.resolve(), args.jars.resolve(), check=check_for(args.seconds),
        server_properties={"view-distance": 6, "max-players": 16}, trace_factory=make_trace,
        server_launcher=(classes.resolve(), "NativeReaderStressControl"))


if __name__ == "__main__": main()
