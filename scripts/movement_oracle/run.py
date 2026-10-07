#!/usr/bin/env python3
"""Run the movement oracle inside each unchanged official server JAR.

The harness source uses Mojang's published names. It is compiled against a
locally remapped copy of the server and its classes are remapped back to the
official obfuscated names, so the game itself runs exactly as distributed.
Nothing derived from the game is written to the repository: only the scenario
results and the hashes of every input.

--downloads must contain, as fetched from Mojang and Maven Central:
  1.16.1-server.jar  1.16.1-server-mappings.txt
  1.21.11-server.jar 1.21.11-server-mappings.txt
  SpecialSource-1.11.4-shaded.jar
"""
import argparse
import hashlib
import json
import shutil
import subprocess
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
HERE = Path(__file__).resolve().parent
SPECIAL_SOURCE_SHA256 = "e2cab24b1c12400ad73b15972bb21e4273a0dc7081c8b3c136ddfdd824c78518"
VERSIONS = {
    # version: (server JAR sha1, mappings sha1, harness directory)
    "1.16.1": ("a412fd69db1f81db3f511c1463fd304675244077", "11120c39da4df293c4bd020896391fb9ddd6c2ba", "java_1_16_1"),
    "1.21.11": ("64bb6d763bed0a9f1d632ec347938594144943ed", "5621e9253f05fd57872bbe7f8ddf5f9a7d525955", "java_1_21_11"),
}
PROPERTIES = """online-mode=false
server-port={port}
level-type={level_type}
generate-structures=false
spawn-npcs=false
spawn-animals=false
spawn-monsters=false
max-tick-time=-1
view-distance=4
sync-chunk-writes=false
"""


def digest(data, name="sha256"):
    return hashlib.new(name, data).hexdigest()


def run(command, cwd, log):
    with log.open("w") as out:
        subprocess.run(command, cwd=cwd, stdout=out, stderr=subprocess.STDOUT, check=True)


def classpath(version, jar, work):
    """The original game classes plus, for the bundled server, its verified libraries."""
    if version == "1.16.1":
        return jar, [jar]
    with zipfile.ZipFile(jar) as bundle:
        libraries = []
        for line in bundle.read("META-INF/libraries.list").decode().splitlines():
            sha, _, path = line.split("\t")
            data = bundle.read("META-INF/libraries/" + path)
            assert digest(data) == sha, path
            target = work / "libraries" / path
            target.parent.mkdir(parents=True, exist_ok=True)
            target.write_bytes(data)
            libraries.append(target)
        sha, _, path = bundle.read("META-INF/versions.list").decode().split("\t")
        data = bundle.read("META-INF/versions/" + path.strip())
        assert digest(data) == sha
        game = work / "server.jar"
        game.write_bytes(data)
    return game, [game] + libraries


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--downloads", type=Path, required=True)
    parser.add_argument("--work", type=Path, required=True)
    parser.add_argument("--scenarios", type=Path, default=HERE / "scenarios.json")
    parser.add_argument("--output", type=Path, default=ROOT / "data/client_api/movement_oracle.json")
    parser.add_argument("--version", choices=sorted(VERSIONS), action="append")
    args = parser.parse_args()
    tool = args.downloads / "SpecialSource-1.11.4-shaded.jar"
    assert digest(tool.read_bytes()) == SPECIAL_SOURCE_SHA256
    scenarios_bytes = args.scenarios.read_bytes()
    scenarios = json.loads(scenarios_bytes)
    results, runs = {}, []
    for port, (version, (jar_sha, mapping_sha, harness)) in enumerate(VERSIONS.items(), start=25650):
        if args.version and version not in args.version:
            continue
        jar = (args.downloads / f"{version}-server.jar").resolve()
        mappings = (args.downloads / f"{version}-server-mappings.txt").resolve()
        assert digest(jar.read_bytes(), "sha1") == jar_sha, jar
        assert digest(mappings.read_bytes(), "sha1") == mapping_sha, mappings
        work = (args.work / version).resolve()
        shutil.rmtree(work, ignore_errors=True)
        (work / "classes").mkdir(parents=True)
        game, runtime = classpath(version, jar, work)
        named = work / "named.jar"
        run(["java", "-jar", str(tool), "--in-jar", str(game), "--out-jar", str(named), "--srg-in", str(mappings)],
            work, work / "remap.log")
        source = HERE / harness / "MovementOracle.java"
        compile_cp = ":".join(str(p) for p in [named] + runtime[1:])
        run(["javac", "-nowarn", "--release", "21", "-d", str(work / "classes"), "-cp", compile_cp, str(source)],
            work, work / "javac.log")
        run(["jar", "cf", str(work / "harness-named.jar"), "-C", str(work / "classes"), "voxrig"], work, work / "jar.log")
        run(["java", "-cp", f"{tool}:{named}", "net.md_5.specialsource.SpecialSource", "--live",
             "--in-jar", str(work / "harness-named.jar"), "--out-jar", str(work / "harness.jar"),
             "--srg-in", str(mappings), "--reverse"], work, work / "unmap.log")
        server = work / "server"
        server.mkdir()
        (server / "eula.txt").write_text("eula=true\n")
        level_type = "flat" if version == "1.16.1" else "minecraft\\:flat"
        (server / "server.properties").write_text(PROPERTIES.format(port=port, level_type=level_type))
        selected = [s for s in scenarios if version in s.get("versions", VERSIONS)]
        (work / "scenarios.json").write_text(json.dumps(selected))
        output = work / "output.json"
        run(["java", "-Xmx1G", "--add-opens", "java.base/java.lang=ALL-UNNAMED", "-cp",
             ":".join(str(p) for p in runtime + [work / "harness.jar"]),
             "voxrig.oracle.MovementOracle", str(work / "scenarios.json"), str(output)], server, work / "server.log")
        data = json.loads(output.read_text())
        assert data["version"] == version
        assert [r["name"] for r in data["results"]] == [s["name"] for s in selected]
        results[version] = data["results"]
        runs.append({"version": version, "original_server_jar_sha1": jar_sha, "mappings_sha1": mapping_sha,
                     "harness_sha256": digest(source.read_bytes())})
    record = {
        "schema": 1,
        "authority": "Official LivingEntity/Player movement run unchanged inside each official server JAR; "
                     "only LocalPlayer's client-side input handling is ported in the harness.",
        "special_source_sha256": SPECIAL_SOURCE_SHA256,
        "runner_sha256": digest(Path(__file__).read_bytes()),
        "scenarios_sha256": digest(scenarios_bytes),
        "runs": runs,
        "scenarios": scenarios,
        "results": results,
    }
    args.output.write_text(json.dumps(record, indent=None, separators=(",", ":")) + "\n")
    print(f"{sum(len(r) for r in results.values())} scenario runs written to {args.output}")


if __name__ == "__main__":
    main()
