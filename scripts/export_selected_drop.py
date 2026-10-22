#!/usr/bin/env python3
"""Export unchanged 1.21.11 drop/resync codecs and no-op menu primitive facts.

The supplied server directory is read-only. Only our observation wrappers are
compiled into a named package; the signed native classes remain unchanged.
No server/world/EULA or network connection is created.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def digest(path, algorithm="sha256"):
    return hashlib.new(algorithm, path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--server-home", required=True, type=Path)
    parser.add_argument("--runtime-output", required=True, type=Path)
    parser.add_argument("--output", type=Path, default=ROOT / "data/client_api")
    args = parser.parse_args()
    home = args.server_home.resolve()
    assert digest(home / "server.jar", "sha1") == "64bb6d763bed0a9f1d632ec347938594144943ed"
    classpath = [home / "versions/1.21.11/server-1.21.11.jar"] + sorted((home / "libraries").rglob("*.jar"))
    runtime = args.runtime_output.resolve()
    runtime.mkdir(parents=True, exist_ok=True)
    classes = runtime / "classes"
    classes.mkdir(exist_ok=True)
    sources = []
    source_hashes = {}
    for name in ("ExportRegularClicks.java", "ExportSelectedDrop.java"):
        source = ROOT / "scripts" / name
        target = runtime / name
        target.write_text("package voxrig_selected_drop;\n" + source.read_text())
        source_hashes["scripts/" + name] = digest(source)
        sources.append(str(target))
    args.output.mkdir(parents=True, exist_ok=True)
    outputs = [args.output.resolve() / ("selected_drop" + suffix + "-1.21.11.json")
               for suffix in ("_packets", "_resync")]
    native_cp = os.pathsep.join(map(str, classpath))
    with (runtime / "oracle.log").open("w") as log:
        subprocess.run(["javac", "-proc:none", "-cp", native_cp, "-d", str(classes), *sources],
                       cwd=runtime, stdout=log, stderr=subprocess.STDOUT, check=True)
        subprocess.run(["java", "-Xmx512M", "-XX:ActiveProcessorCount=1", "-cp",
                        str(classes) + os.pathsep + native_cp, "voxrig_selected_drop.ExportSelectedDrop",
                        *map(str, outputs)], cwd=runtime, stdout=log, stderr=subprocess.STDOUT, check=True)
    source_hashes["scripts/export_selected_drop.py"] = digest(Path(__file__))
    evidence = dict(server_jar_sha1=digest(home / "server.jar", "sha1"),
                    sources=source_hashes,
                    original_classpath_entries_sha256={str(p.relative_to(home)): digest(p) for p in classpath},
                    outputs_sha256={p.name: digest(p) for p in outputs})
    # Codec/primitive facts are distinct from live network and ownership evidence.
    (runtime / "source.json").write_text(json.dumps(evidence, indent=2) + "\n")
    print("PASS: 10 drop codec cases; resync codec; 3 unchanged native slot/cursor cases")


if __name__ == "__main__":
    main()
