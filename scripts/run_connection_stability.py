#!/usr/bin/env python3
"""Repeated common-API disconnect during initial vanilla chunk loading (#20)."""
import argparse
import time
from pathlib import Path
from run_climbing_control import run
from run_common_native import until


def check(version, command, request, trace, report):
    for trial in range(10):
        name = f"Disconnect{trial}"
        connected = request("connect", name=name)
        assert connected["connected"]
        index = connected["index"]
        observations = []
        for _ in range(2):
            time.sleep(1)
            observations.append(request("read", index=index))
        trace.expect_disconnect()
        started = time.monotonic()
        assert request("disconnect", index=index) == {"disconnected": True}
        elapsed = time.monotonic() - started
        until(lambda: "Test passed" in command(f"execute unless entity @a[name={name}]"))
        report["checks"].append(dict(trial=trial, name=name, observations=observations,
                                      disconnect_seconds=elapsed, disconnected=True))
    request("quit")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--jars", type=Path)
    parser.add_argument("--version", choices=["1.16.1", "1.21.11"], action="append")
    args = parser.parse_args()
    for version in args.version or ["1.16.1", "1.21.11"]:
        run(version, args.binary.resolve(), args.jars.resolve() if args.jars else None,
            check=check, server_properties={"view-distance": 6, "max-players": 8})


if __name__ == "__main__":
    main()
