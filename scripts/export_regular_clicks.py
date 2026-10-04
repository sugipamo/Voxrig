#!/usr/bin/env python3
"""Export unmodified official PICKUP/SWAP primitives and cursor-comparison codecs.

Obtain the pinned official server JARs/mappings separately. Modern classpath must
contain the original bundled server and its original libraries. Runs one JVM at
a time, with 512 MiB heap and one active processor; never starts a game server.
"""
import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path
import subprocess
import zipfile

ROOT = Path(__file__).resolve().parents[1]
VERSIONS = {
    "1.16.1": ("a412fd69db1f81db3f511c1463fd304675244077", "9b19cc7d56e58258117e2fcc092f773b151682ecb2b1adb3de9832b3c450a30f"),
    "1.21.11": ("64bb6d763bed0a9f1d632ec347938594144943ed", "7ffd98f77f403043748d56467aac287a58f8187a27d5d6b7ebcac70042b02b1c"),
}

def digest(data, kind="sha256"):
    return hashlib.new(kind, data).hexdigest()

def encoded(value, compact=False):
    return (json.dumps(value, sort_keys=True, **({"separators": (",", ":")} if compact else {"indent": 2})) + "\n").encode()

def original_modern_classpath(jar, classpath):
    with zipfile.ZipFile(jar) as bundle:
        expected = {}
        for group in ("versions", "libraries"):
            for line in bundle.read(f"META-INF/{group}.list").decode().splitlines():
                sha, _, path = line.split("\t")
                if digest(bundle.read(f"META-INF/{group}/{path}")) != sha:
                    raise SystemExit("original bundle entry hash mismatch")
                expected[sha] = path
    actual = {}
    for entry in classpath.split(os.pathsep):
        sha = digest(Path(entry).read_bytes())
        if sha not in expected:
            raise SystemExit(f"classpath entry is not an unchanged original bundle entry: {entry}")
        actual[sha] = expected[sha]
    if actual != expected:
        raise SystemExit("modern classpath does not contain every original bundled entry")
    return actual

def normalize(raw):
    policies, menus = [], []
    for menu in raw["menus"]:
        slots = []
        for slot in menu["slots"]:
            policy = {key: slot[key] for key in ("native_class", "may_pickup", "base_capacity", "rejected_default_items")}
            if policy not in policies:
                policies.append(policy)
            value = {"slot": slot["slot"], "policy": policies.index(policy)}
            if "raw_player_slot" in slot:
                value["raw_player_slot"] = slot["raw_player_slot"]
            slots.append(value)
        menus.append({"name": menu["name"], "slots": slots})
    return {"schema": 1, "version": raw["version"], "items": raw["items"], "slot_policies": policies, "menus": menus}

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--downloads", type=Path, required=True, help="VERSION-server.jar and VERSION-server-mappings.txt")
    parser.add_argument("--modern-classpath-file", type=Path, required=True)
    parser.add_argument("--runtime-output", type=Path, required=True)
    parser.add_argument("--normalize-only", action="store_true", help="normalize already exported raw oracle files")
    parser.add_argument("--check", action="store_true", help="compare all generated files to committed evidence")
    args = parser.parse_args()
    args.runtime_output.mkdir(parents=True, exist_ok=True)
    outputs, runs = {}, []
    # Resolve before moving the JVM's cwd to its disposable diagnostic directory.
    modern_classpath = os.pathsep.join(str(Path(p).resolve()) for p in args.modern_classpath_file.read_text().strip().split(os.pathsep))
    for version, (jar_sha1, mappings_sha256) in VERSIONS.items():
        jar = args.downloads / f"{version}-server.jar"
        mappings = args.downloads / f"{version}-server-mappings.txt"
        if digest(jar.read_bytes(), "sha1") != jar_sha1 or digest(mappings.read_bytes()) != mappings_sha256:
            raise SystemExit(f"original JAR/mappings hash differs: {version}")
        classpath_hashes = original_modern_classpath(jar, modern_classpath) if version == "1.21.11" else {digest(jar.read_bytes()): jar.name}
        raw_path = args.runtime_output / f"regular-click-{version}-raw.json"
        if not args.normalize_only:
            classpath = str(jar.resolve()) if version == "1.16.1" else modern_classpath
            with (args.runtime_output / f"regular-click-{version}-export.log").open("w") as log:
                subprocess.run(["java", "-Xmx512M", "-XX:ActiveProcessorCount=1", "-cp", classpath,
                                str(ROOT / "scripts/ExportRegularClicks.java"), version, str(raw_path.resolve())],
                               cwd=args.runtime_output.resolve(), stdout=log, stderr=subprocess.STDOUT, check=True)
        raw = json.loads(raw_path.read_text())
        if raw["version"] != version or len(raw["items"]) != (975 if version == "1.16.1" else 1505) or len(raw["menus"]) != 10:
            raise SystemExit(f"native registry/layout coverage differs: {version}")
        if len(raw["cases"]) != 18432 or len(raw["swaps"]) != 7500:
            raise SystemExit(f"native primitive coverage differs: {version}")
        packets_key = "legacy_packets" if version == "1.16.1" else "modern_packets"
        if len(raw[packets_key]) != 120:
            raise SystemExit(f"native codec coverage differs: {version}")
        profile_path = f"data/client_api/regular_click_profiles-{version}.json"
        fixture_path = f"data/client_api/regular_click_cases-{version}.json.gz"
        packet_path = f"data/client_api/regular_click_packets-{version}.json"
        outputs[profile_path] = encoded(normalize(raw))
        outputs[fixture_path] = gzip.compress(encoded({"version": version, "cases": raw["cases"], "swaps": raw["swaps"]}, compact=True), mtime=0)
        outputs[packet_path] = encoded(raw[packets_key])
        runs.append({"version": version, "original_server_jar_sha1": jar_sha1, "mappings_sha256": mappings_sha256,
                     "original_classpath_entries_sha256": classpath_hashes,
                     "raw_output_sha256": digest(raw_path.read_bytes()), "item_profiles": len(raw["items"]), "slot_layouts": 10,
                     "pickup_cases": 18432, "valid_count_pickup_cases": sum(c["valid_default_counts"] for c in raw["cases"]),
                     "valid_requested_count_pickup_cases": sum(c["valid_requested_counts"] for c in raw["cases"]),
                     "swap_cases": 7500, "codec_roundtrips": 120,
                     "files_sha256": {p: digest(outputs[p]) for p in (profile_path, fixture_path, packet_path)}})
    manifest = {"schema": 1,
                "authority": "Actual unchanged native menu clicked, Slot mayPlace/mayPickup/capacity, Item override declarations, default ItemStack capacities and original packet/hash codecs on pinned official server JARs. No native method bodies redistributed.",
                "context": "Unspawned skeletal ServerPlayer with real native Inventory; modern native ServerLevel/DedicatedServer getters expose actual DEFAULT_FLAGS through a WorldData interface proxy. Only ordinary player slots 9..44 and audited storage slots are queried. This is a primitive/codec oracle, not network/session/mode/world/ownership/recovery validation.",
                "scope": "PICKUP left/right (empty, half, one, merge, exchange, no-op, slot refusal); five sample item classes with capacities 64/16/1. Requested fixture stacks and actual predecessor after unchanged native slot/cursor setters are separate; native fixture clamping is retained, not replaced. Requested-count and actual-count validity are distinct; invalid overstacks are not manually normalized. All registered items inspected for default limits and native item overrides. AIR represents empty; its sentinel stack capacity is not an item-capacity definition. Bundle overrides are excluded from ordinary PICKUP. ShulkerBoxSlot's complete native default-item refusal set is explicit. Valid-count SWAP separately exercises both hotbar ends and slot refusal. Legacy signed window codec samples stop at 127; modern includes 128.",
                "generators_sha256": {p: digest((ROOT / p).read_bytes()) for p in ("scripts/ExportRegularClicks.java", "scripts/export_regular_clicks.py")},
                "upstream_inputs_sha256": {"data/items.json": digest((ROOT / "data/items.json").read_bytes())},
                "effective_capacity_corrections": [{"version": "1.16.1", "native_id": 842, "name": "minecraft:warped_fungus_on_a_stick", "upstream_capacity": 64, "native_capacity": 1, "application": "registry loader uses native capacities; upstream data/items.json is preserved unchanged for its harvest audit source hash"}],
                "runs": runs}
    outputs["data/client_api/regular_click_source.json"] = encoded(manifest)
    for path, data in outputs.items():
        target = ROOT / path
        if args.check:
            if target.read_bytes() != data:
                raise SystemExit(f"generated native click data differs: {path}")
        else:
            target.write_bytes(data)
    print("native click evidence verified" if args.check else "native click evidence generated")

if __name__ == "__main__":
    main()
