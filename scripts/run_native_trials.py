#!/usr/bin/env python3
"""Own the three ignored Java 1.21.11 mining trials, one fresh world at a time.

Run from a checkout with Java 21 and Rust installed:
  python3 -B scripts/run_native_trials.py --accept-eula --server-jar /path/server.jar

The jar (including a supplied jar) is SHA1 verified against Mojang metadata.
All mutable server state and retained diagnostics live in a printed tempfile
folder. Nothing is uploaded. Ctrl-C/SIGTERM stop owned process groups; logs and
partial trial output survive failure. No mining action or edit is retried.
"""
import argparse
from contextlib import ExitStack
from datetime import datetime, timezone
import hashlib
import json
import math
import os
from pathlib import Path
import secrets
import signal
import socket
import subprocess
import tempfile
import time
import traceback

from run_common_native import REPO, Rcon, download, free_port, until

SPEC = REPO / "scripts/fixtures/native_trials_1_21_11.json"
MINING = ("mining_raw", "mining_api", "mining_recovery")
IDENTITY = ("schema_version", "run_id", "trial_id", "case_id", "request_id", "phase")
REQUEST_FIELDS = set(IDENTITY) | {"game_port", "action_elapsed_ms"}


def save(path, value):
    temporary = path.with_suffix(path.suffix + ".tmp")
    temporary.write_text(json.dumps(value, indent=2) + "\n")
    temporary.replace(path)


class Journal:
    def __init__(self, stream):
        self.stream = stream

    def emit(self, stage, **fields):
        self.stream.write(json.dumps({"time": datetime.now(timezone.utc).isoformat(),
                                      "stage": stage, **fields}) + "\n")
        self.stream.flush()


def frame(line):
    if len(line) > 4096 or not line.endswith(b"\n"):
        raise ValueError("invalid control frame")
    value = json.loads(line)
    if not isinstance(value, dict) or set(value) != REQUEST_FIELDS:
        raise ValueError("invalid control request fields")
    return value


class Peer:
    """Synchronous fail-closed control peer; RCON is injected for pure tests."""
    def __init__(self, trial, port, rcon, journal, clock=time.monotonic):
        self.trial, self.port, self.rcon = trial, port, rcon
        self.journal, self.clock = journal, clock
        self.run_id = None
        self.next_request = 1
        self.case_index = -1
        self.edited = set()
        self.usable = True

    def command(self, command):
        response = self.rcon.command(command)
        self.journal.emit("rcon", command=command, response=response)
        if any(s in response.lower() for s in (
                "unknown or incomplete", "incorrect argument", "not loaded",
                "no player was found", "unknown item", "invalid")):
            raise RuntimeError(f"fixture command rejected: {command}: {response}")
        return response

    def handle(self, request):
        if not self.usable:
            raise ValueError("control peer unusable after failure")
        self.usable = False  # A failed request may have applied edits; never retry.
        started = self.clock()
        self.journal.emit("request", request=request)
        if set(request) != REQUEST_FIELDS:
            raise ValueError("invalid request fields")
        if (type(request["schema_version"]) is not int or request["schema_version"] != 1
                or request["trial_id"] != self.trial["id"]
                or type(request["game_port"]) is not int or request["game_port"] != self.port
                or type(request["request_id"]) is not int
                or request["request_id"] != self.next_request
                or not isinstance(request["run_id"], str) or not request["run_id"]
                or (self.run_id is not None and request["run_id"] != self.run_id)):
            raise ValueError("stale, duplicate or mismatched control identity")
        cases = self.trial["cases"]
        phase = request["phase"]
        elapsed = request["action_elapsed_ms"]
        if phase == "setup":
            index = self.case_index + 1
            if (index >= len(cases) or cases[index]["id"] != request["case_id"]
                    or elapsed is not None):
                raise ValueError("out-of-order setup")
            if self.case_index >= 0 and not self.case_complete():
                raise ValueError("previous case missing external edit")
            case = cases[index]
            if case["setup"] != "reset_before_case":
                raise ValueError("unsupported setup policy")
            for region in case["regions"]:
                coords = " ".join(map(str, region["min"] + region["max"]))
                self.command(f'fill {coords} minecraft:{region["material"]}')
            self.command("kill @e[type=minecraft:item]")
            for profile in self.trial["profiles"]:
                name = profile["name"]
                self.command(f"gamemode survival {name}")
                self.command(f"clear {name}")
                coords = " ".join(map(str, profile["position"]))
                self.command(f"tp {name} {coords} 0 0")
                self.command(f"spawnpoint {name} 0 -60 0")
            for supply in self.trial["inventory"]:
                slot = supply["slot"]
                native = f"inventory.{slot - 9}" if slot < 36 else f"hotbar.{slot - 36}"
                self.command(f'item replace entity NatMineBot {native} with '
                             f'minecraft:{supply["item"]} {supply["count"]}')
            for profile in self.trial["profiles"]:
                for field in ("Pos", "playerGameType", "Health"):
                    self.command(f'data get entity {profile["name"]} {field}')
            self.command("data get entity NatMineBot Inventory")
            self.case_index = index
        elif phase == "external_edit":
            if self.case_index < 0 or cases[self.case_index]["id"] != request["case_id"]:
                raise ValueError("edit before matching setup")
            case = cases[self.case_index]
            edits = [e for e in case["permitted_external_edits"] if e["phase"] == phase]
            if len(edits) != 1 or case["id"] in self.edited:
                raise ValueError("undeclared or duplicate external edit")
            edit = edits[0]
            deadline = edit["deadline_from_action_ms"]
            if (type(elapsed) not in (int, float) or not math.isfinite(elapsed)
                    or elapsed < 0 or elapsed >= deadline):
                raise ValueError("external edit deadline already expired or invalid")
            # Bound RCON by the remaining action budget, rather than a new 3s timer.
            self.rcon.stream.settimeout((deadline - elapsed) / 1000)
            coords = " ".join(map(str, edit["target"]))
            for material in edit["materials"]:
                self.command(f"setblock {coords} minecraft:{material}")
            completion = elapsed + (self.clock() - started) * 1000
            self.journal.emit("external_timing", action_elapsed_at_request_ms=elapsed,
                              controller_completion_ms=completion, deadline_ms=deadline)
            if completion >= deadline:
                raise TimeoutError("external edit exceeded action deadline")
            self.rcon.stream.settimeout(10)
            self.edited.add(case["id"])
        else:
            raise ValueError("undeclared control phase")
        self.run_id = request["run_id"]
        self.next_request += 1
        reply = {key: request[key] for key in IDENTITY}
        reply["status"] = "ready"
        self.journal.emit("reply", reply=reply, processing_ms=(self.clock() - started) * 1000)
        self.usable = True
        return reply

    def case_complete(self):
        case = self.trial["cases"][self.case_index]
        return not case["permitted_external_edits"] or case["id"] in self.edited

    def complete(self):
        return (self.usable and self.case_index == len(self.trial["cases"]) - 1
                and self.case_complete())


def control_loop(listener, process, peer, timeout):
    """Serve one connection while monitoring Cargo, including before connection."""
    deadline = time.monotonic() + timeout
    listener.settimeout(0.2)
    stream = None
    pending = b""
    try:
        while True:
            if time.monotonic() >= deadline:
                raise TimeoutError("trial process deadline exceeded")
            if process.poll() is not None:
                return process.returncode
            if stream is None:
                try:
                    stream, _ = listener.accept()
                except socket.timeout:
                    continue
                stream.settimeout(0.2)
                peer.journal.emit("control_connected")
                continue
            try:
                data = stream.recv(4097)
            except socket.timeout:
                continue
            if not data:
                if pending:
                    raise ValueError("truncated control frame")
                # EOF is expected when the Rust test exits, including panic.
                return process.wait(timeout=max(0.1, deadline - time.monotonic()))
            pending += data
            while b"\n" in pending:
                line, pending = pending.split(b"\n", 1)
                reply = peer.handle(frame(line + b"\n"))
                stream.sendall((json.dumps(reply) + "\n").encode())
            if len(pending) >= 4096:
                raise ValueError("oversized control frame")
    finally:
        if stream is not None:
            stream.close()


def stop_group(process):
    if process is None:
        return
    # Cargo's test executable is a descendant and must also be stopped on panic.
    for sig, seconds in ((signal.SIGTERM, 10), (signal.SIGKILL, 5)):
        try:
            os.killpg(process.pid, sig)
        except ProcessLookupError:
            break
        try:
            process.wait(timeout=seconds)
        except subprocess.TimeoutExpired:
            continue
        if sig == signal.SIGTERM:
            # A reaped parent alone does not establish that its children exited.
            continue
    process.wait(timeout=5)


def configure(folder, port, rcon_port, password, accept_eula):
    if not accept_eula:
        raise ValueError("--accept-eula is required to start a disposable server")
    (folder / "eula.txt").write_text("eula=true\n")
    settings = json.dumps({"biome": "minecraft:plains", "layers": [
        {"block": "minecraft:bedrock", "height": 1},
        {"block": "minecraft:dirt", "height": 2},
        {"block": "minecraft:grass_block", "height": 1}], "structure_overrides": []})
    properties = {
        "server-ip": "127.0.0.1", "server-port": port, "level-name": "world",
        "level-type": "minecraft:flat", "generator-settings": settings, "level-seed": 1234,
        "online-mode": "false", "gamemode": "survival", "force-gamemode": "true",
        "spawn-protection": 0, "view-distance": 3, "simulation-distance": 3,
        "max-players": 4, "max-tick-time": 60000, "enable-rcon": "true",
        "rcon.port": rcon_port, "rcon.password": password, "broadcast-rcon-to-ops": "false",
        "enable-status": "false", "sync-chunk-writes": "false",
        "network-compression-threshold": 256, "difficulty": "peaceful",
    }
    path = folder / "server.properties"
    path.touch(mode=0o600)
    path.write_text("".join(f"{k}={v}\n" for k, v in properties.items()))
    (folder / "ops.json").write_text("[]\n")
    (folder / "whitelist.json").write_text("[]\n")


def run_trial(trial, jar, folder, accept_eula):
    folder.mkdir()
    report = {"trial_id": trial["id"], "result": "running", "folder": str(folder)}
    server = cargo = rcon = None
    with ExitStack() as stack:
        journal = Journal(stack.enter_context((folder / "journal.jsonl").open("w")))

        def stage(name, **fields):
            report.update(stage=name, **fields)
            journal.emit(name, **fields)
            save(folder / "report.json", report)

        try:
            ports = set()
            while len(ports) < 2:
                candidate = free_port()
                if candidate != 25572:
                    ports.add(candidate)
            port, rcon_port = sorted(ports)
            password = secrets.token_hex(24)
            configure(folder, port, rcon_port, password, accept_eula)
            listener = stack.enter_context(socket.socket())
            listener.bind(("127.0.0.1", 0))
            if listener.getsockname()[1] == 25572:
                raise RuntimeError("reserved control port allocated; rerun before any action")
            listener.listen(1)
            server_log = stack.enter_context((folder / "server.log").open("w"))
            stage("server_start", game_port=port, rcon_port=rcon_port)
            server = subprocess.Popen([
                "java", "-XX:ActiveProcessorCount=2", "-Xms256M", "-Xmx1G",
                "-jar", str(jar), "nogui"], cwd=folder, stdin=subprocess.PIPE,
                stdout=server_log, stderr=subprocess.STDOUT, text=True, start_new_session=True)
            stage("server_started", server_pid=server.pid)

            def connect():
                if server.poll() is not None:
                    raise RuntimeError("server exited during bootstrap; see server.log")
                try:
                    return Rcon(rcon_port, password)
                except OSError:
                    return None

            rcon = until(connect, 180)
            peer = Peer(trial, port, rcon, journal)
            peer.command("forceload add -16 -16 16 16")
            until(lambda: "Test passed" in peer.command("execute if loaded 0 -61 0"), 60)
            for command in (
                    "gamerule minecraft:respawn_radius 0", "gamerule minecraft:spawn_mobs false",
                    "gamerule minecraft:advance_time false", "weather clear", "time set noon"):
                peer.command(command)
            # Bootstrap the spawn floor/air from the spec before either profile joins.
            for region in trial["cases"][0]["regions"]:
                if region["role"] in ("floor", "air"):
                    coords = " ".join(map(str, region["min"] + region["max"]))
                    peer.command(f'fill {coords} minecraft:{region["material"]}')
            peer.command("setworldspawn 0 -60 0")
            stage("server_ready")
            endpoint = f"127.0.0.1:{listener.getsockname()[1]}"
            env = dict(os.environ, NATIVE_TRIAL_CONTROL=endpoint)
            env[trial["port_env"]] = str(port)
            env[trial["output_env"]] = str(folder / "trial.json")
            name = "versions::java_1_21_11::client::mining_native_trials::" + trial["test"]
            command = ["cargo", "+stable", "test", "--locked", "--lib", "--features", "native",
                       "-j2", name, "--", "--exact", "--ignored", "--nocapture", "--test-threads=1"]
            test_log = stack.enter_context((folder / "test.log").open("w"))
            stage("trial_start", command=command, control=endpoint)
            cargo = subprocess.Popen(command, cwd=REPO, env=env, stdin=subprocess.DEVNULL,
                                     stdout=test_log, stderr=subprocess.STDOUT, start_new_session=True)
            stage("trial_started", cargo_pid=cargo.pid)
            code = control_loop(listener, cargo, peer, 1200)
            test_log.flush()
            lines = (folder / "test.log").read_text().splitlines()
            results = [line for line in lines if line.startswith("test result:")]
            stage("trial_exited", exit_code=code, test_result_lines=results)
            if code != 0 or not peer.complete() or not any("1 passed; 0 failed" in s for s in results):
                raise RuntimeError("trial failed or control phases incomplete; see test.log/journal.jsonl")
            evidence = json.loads((folder / "trial.json").read_text())
            if len(evidence["cases"]) != len(trial["cases"]):
                raise RuntimeError("incomplete trial evidence")
            report["result"] = "PASS"
        except BaseException:
            report.update(result="FAIL", error=traceback.format_exc())
            raise
        finally:
            # Finish reaping even if a second signal arrives during shutdown.
            handlers = {sig: signal.signal(sig, signal.SIG_IGN)
                        for sig in (signal.SIGINT, signal.SIGTERM)}
            try:
                stage("cleanup")
            finally:
                # A journal write failure must not strand an owned process.
                try:
                    stop_group(cargo)
                finally:
                    try:
                        if server is not None and server.poll() is None:
                            try:
                                server.stdin.write("stop\n")
                                server.stdin.flush()
                                server.wait(timeout=60)
                            except (OSError, subprocess.TimeoutExpired):
                                pass
                        stop_group(server)
                    finally:
                        if rcon is not None:
                            rcon.close()
                        if server is not None and server.stdin is not None:
                            server.stdin.close()
                        if report["result"] == "PASS" and server.returncode != 0:
                            report.update(result="FAIL", error="server did not exit cleanly")
                        try:
                            stage("stopped", server_exit_code=server.returncode if server else None)
                        finally:
                            for sig, handler in handlers.items():
                                signal.signal(sig, handler)
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--accept-eula", action="store_true")
    parser.add_argument("--server-jar", type=Path)
    parser.add_argument("--trial", choices=MINING, action="append", help="default: all three, sequentially")
    args = parser.parse_args()
    if not args.accept_eula:
        parser.error("--accept-eula is required; no server files have been written")
    root = Path(tempfile.mkdtemp(prefix="voxrig-native-mining-"))
    print(f"Retained run directory: {root}", flush=True)
    summary = {"result": "running", "trials": []}

    def interrupted(signum, _frame):
        # Ignore repeated termination while finally blocks reap owned processes.
        signal.signal(signal.SIGTERM, signal.SIG_IGN)
        signal.signal(signal.SIGINT, signal.SIG_IGN)
        raise KeyboardInterrupt(f"signal {signum}")

    previous = {sig: signal.signal(sig, interrupted) for sig in (signal.SIGINT, signal.SIGTERM)}
    try:
        summary.update(
            revision=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=REPO, text=True).strip(),
            java=subprocess.run(["java", "-version"], capture_output=True, text=True, check=True).stderr,
            fixture_sha256=hashlib.sha256(SPEC.read_bytes()).hexdigest())
        save(root / "report.json", summary)
        # A new metadata directory prevents trusting an old cached source record.
        jar, source = download("1.21.11", downloads=root / "downloads",
                               server_jar=args.server_jar.resolve() if args.server_jar else None)
        save(root / "source.json", source)
        spec = json.loads(SPEC.read_text())
        for trial_id in args.trial or MINING:
            trial = next(t for t in spec["trials"] if t["id"] == trial_id)
            folder = root / trial_id
            print(f"Running {trial_id}: {folder}", flush=True)
            try:
                result = run_trial(trial, jar, folder, args.accept_eula)
            finally:
                if (folder / "report.json").exists():
                    summary["trials"].append(json.loads((folder / "report.json").read_text()))
                    save(root / "report.json", summary)
            print(f'{trial_id}: {result["result"]} {result["test_result_lines"]}', flush=True)
            if result["result"] != "PASS":
                raise RuntimeError("trial qualification failed")
        summary["result"] = "PASS"
        return 0
    except BaseException:
        summary.update(result="FAIL", error=traceback.format_exc())
        traceback.print_exc()
        return 1
    finally:
        save(root / "report.json", summary)
        for sig, handler in previous.items():
            signal.signal(sig, handler)


if __name__ == "__main__":
    raise SystemExit(main())
