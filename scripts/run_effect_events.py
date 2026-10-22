#!/usr/bin/env python3
"""Verify Java 1.21.11 transient events on a disposable local official server."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import queue
import shutil
import socket
import subprocess
import tempfile
import threading
import time


def free_port():
    while True:
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            port = listener.getsockname()[1]
        if port not in (25572, 25581):
            return port


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--accept-eula", action="store_true", required=True)
    parser.add_argument("--server-jar", type=Path, required=True)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    out = args.output.resolve()
    out.mkdir(parents=True, exist_ok=True)
    port = free_port()
    run = Path(tempfile.mkdtemp(prefix="voxrig-effects-"))
    shutil.copyfile(args.server_jar.resolve(), run / "server.jar")
    jar_sha1 = hashlib.sha1((run / "server.jar").read_bytes()).hexdigest()
    if jar_sha1 != "64bb6d763bed0a9f1d632ec347938594144943ed":
        shutil.rmtree(run)
        raise ValueError("expected the pinned official Java 1.21.11 server JAR")
    (run / "eula.txt").write_text("eula=true\n")
    properties = {
        "server-ip": "127.0.0.1", "server-port": port,
        "online-mode": "false", "enforce-secure-profile": "false",
        "spawn-protection": 0, "view-distance": 2, "simulation-distance": 2,
        "max-players": 4, "level-type": "minecraft:flat",
        "generator-settings": json.dumps({
            "layers": [{"block": "minecraft:bedrock", "height": 1},
                       {"block": "minecraft:dirt", "height": 2},
                       {"block": "minecraft:grass_block", "height": 1}],
            "biome": "minecraft:plains",
        }),
        "sync-chunk-writes": "false",
    }
    (run / "server.properties").write_text(
        "".join(f"{key}={value}\n" for key, value in properties.items()))
    messages = queue.Queue()
    server_log = (out / "live-server.log").open("w")
    server = subprocess.Popen(
        ["java", "-Xms256M", "-Xmx768M", "-XX:ActiveProcessorCount=2",
         "-jar", "server.jar", "nogui"],
        cwd=run, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT, text=True, bufsize=1)

    def pump():
        for line in server.stdout:
            server_log.write(line)
            server_log.flush()
            messages.put(line)

    thread = threading.Thread(target=pump, daemon=True)
    thread.start()
    probe = timer = None
    try:
        deadline = time.monotonic() + 120
        while True:
            line = messages.get(timeout=max(0.1, deadline - time.monotonic()))
            if "Done (" in line:
                break
            if time.monotonic() >= deadline:
                raise TimeoutError("server start timeout")
        env = dict(os.environ, VOXRIG_PORT=str(port),
                   VOXRIG_EFFECT_TRACE=str(out / "live-trace.json"))
        probe = subprocess.Popen(
            [str(args.binary.resolve())], env=env, stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT, text=True, bufsize=1)
        timer = threading.Timer(150, probe.kill)
        timer.start()
        observations, commands = [], []
        with (out / "live-probe.log").open("w") as log:
            for line in probe.stdout:
                log.write(line)
                log.flush()
                print(line, end="", flush=True)
                if line.startswith("CMD "):
                    command = line[4:].rstrip()
                    commands.append(command)
                    server.stdin.write(command + "\n")
                    server.stdin.flush()
                if line.startswith("OBS "):
                    observations.append(json.loads(line[4:]))
        result = probe.wait()
        report = dict(version="1.21.11", server_jar_sha1=jar_sha1, port=port,
                      exit_code=result, commands=commands, observations=observations)
        (out / "live-result.json").write_text(json.dumps(report, indent=2) + "\n")
        if result != 0:
            raise RuntimeError("effect probe failed")
    finally:
        if timer is not None:
            timer.cancel()
        if probe is not None and probe.poll() is None:
            probe.kill()
            probe.wait()
        if server.poll() is None:
            try:
                server.stdin.write("stop\n")
                server.stdin.flush()
                server.wait(timeout=25)
            except (BrokenPipeError, subprocess.TimeoutExpired):
                server.terminate()
                try:
                    server.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    server.kill()
                    server.wait()
        thread.join(timeout=5)
        server_log.close()
        shutil.rmtree(run)
        print(f"Server stopped: exit={server.returncode}, port={port}", flush=True)


if __name__ == "__main__":
    main()
