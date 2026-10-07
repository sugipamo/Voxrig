#!/usr/bin/env python3
"""Observe original title/tab-list/border codecs, sequential 512 MiB JVMs."""
import argparse
import os
import subprocess
from pathlib import Path
from export_regular_clicks import ROOT, VERSIONS, digest, encoded, original_modern_classpath


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ["downloads", "modern-classpath-file", "runtime-output"]:
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--normalize-only", action="store_true")
    parser.add_argument("--check", action="store_true")
    args = parser.parse_args()
    base = args.runtime_output.resolve()
    base.mkdir(parents=True, exist_ok=True)
    cp = os.pathsep.join(str(Path(p).resolve()) for p in args.modern_classpath_file.read_text().strip().split(os.pathsep))
    facts = []
    runs = []
    import json
    for version, (jar_sha, mapping_sha) in VERSIONS.items():
        jar = (args.downloads / (version + "-server.jar")).resolve()
        mapping = args.downloads / (version + "-server-mappings.txt")
        assert digest(jar.read_bytes(), "sha1") == jar_sha and digest(mapping.read_bytes()) == mapping_sha
        runtime = str(jar) if version == "1.16.1" else cp
        classpath = {digest(jar.read_bytes()): jar.name} if version == "1.16.1" else original_modern_classpath(jar, cp)
        raw = base / (version + "-codec.json")
        if not args.normalize_only:
            with (base / (version + "-codec.log")).open("w") as log:
                subprocess.run(["java", "-Xmx512M", "-XX:ActiveProcessorCount=1", "-cp", runtime,
                                str(ROOT / "scripts/ExportDisplayPackets.java"), version, str(raw)],
                               stdout=log, stderr=subprocess.STDOUT, check=True)
        data = json.loads(raw.read_text())
        assert data["version"] == version and len(data["packets"]) == 21
        facts.append(data)
        runs.append({"version": version, "original_server_jar_sha1": jar_sha, "mappings_sha256": mapping_sha,
                     "original_classpath_entries_sha256": classpath, "raw_output_sha256": digest(raw.read_bytes())})
    name = "data/client_api/display_packets.json"
    outputs = {name: encoded({"schema": 1, "versions": facts})}
    outputs["data/client_api/display_source.json"] = encoded({
        "schema": 1,
        "authority": "Unchanged official title/tab-list/border readers/writers and reflected native decoded fields; no game bodies copied or replaced.",
        "scope": "Title text/subtitle/action bar/signed timings/clear/reset; complete header-footer pair; all border operations and wide signed durations. Original literal text samples. Separate from cache/session atomicity, rendering, interpolation/collision/damage and live workflow checks.",
        "generators_sha256": {p: digest((ROOT / p).read_bytes()) for p in ["scripts/ExportDisplayPackets.java", "scripts/export_display_packets.py"]},
        "runs": runs, "files_sha256": {name: digest(outputs[name])}})
    for name, value in outputs.items():
        if args.check:
            assert (ROOT / name).read_bytes() == value, name
        else:
            (ROOT / name).write_bytes(value)
    print("original display codecs verified" if args.check else "original display codecs generated")


if __name__ == "__main__":
    main()
