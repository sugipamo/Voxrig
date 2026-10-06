#!/usr/bin/env python3
"""Observe original mounted-input and passenger codecs, sequential 512 MiB JVMs."""
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
                                str(ROOT / "scripts/ExportVehicleInput.java"), version, str(raw)],
                               stdout=log, stderr=subprocess.STDOUT, check=True)
        data = json.loads(raw.read_text())
        assert data["version"] == version and len(data["inputs"]) == 2 and len(data["passengers"]) == 3
        facts.append(data)
        runs.append({"version": version, "original_server_jar_sha1": jar_sha, "mappings_sha256": mapping_sha,
                     "original_classpath_entries_sha256": classpath, "raw_output_sha256": digest(raw.read_bytes())})
    name = "data/client_api/vehicle_input_packets.json"
    outputs = {name: encoded({"schema": 1, "versions": facts})}
    outputs["data/client_api/vehicle_input_source.json"] = encoded({
        "schema": 1,
        "authority": "Original unmodified mounted-input/passenger packet readers and writers; reflected original decoded fields. Legacy client-side convenience constructors/getters stripped from server JAR are not replaced: legacy input is populated by its original native reader. Modern uses original Input/packet constructors and stream codec.",
        "scope": "Two neutral-axis/default-input shift states and three passenger arrays only; no riding ticks, vehicle physics, session/mode/lifetime/cancellation, release timing or live-server outcome proof.",
        "generators_sha256": {p: digest((ROOT / p).read_bytes()) for p in ["scripts/ExportVehicleInput.java", "scripts/export_vehicle_input.py"]},
        "runs": runs, "files_sha256": {name: digest(outputs[name])}})
    for name, value in outputs.items():
        if args.check:
            assert (ROOT / name).read_bytes() == value, name
        else:
            (ROOT / name).write_bytes(value)
    print("original mounted-input/passenger codecs verified" if args.check else "original mounted-input/passenger codecs generated")


if __name__ == "__main__":
    main()
