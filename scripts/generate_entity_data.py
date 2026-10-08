#!/usr/bin/env python3
"""Turn ExportEntityData.java output into data/<version>/entity_data.json.

Inputs per version: the exporter output and the official server mappings. A
type's registry name is its lower-cased EntityType constant (EntityType.CREEPER is
minecraft:creeper); every other name in the output is the Mojang-mapped name. Each
row is [owner class, field, index, default kind, default value].

For a type whose defineSynchedData stopped early (registry-backed variants need a
world), pass `javap -c -p` of each such class as <obf>.txt in [javap-dir]: constant
defines in its defineSynchedData fill the remaining defaults.

usage: generate_entity_data.py <export.txt> <server-mappings.txt> <out.json> <minecraft> [javap-dir]
"""
import json
import os
import re
import sys


def mappings(path):
    classes, fields, current = {}, {}, None
    for line in open(path, encoding="utf-8"):
        if line.startswith("#"):
            continue
        if not line.startswith(" "):
            named, obf = line.rstrip(":\n").split(" -> ")
            classes[obf] = named
            current = obf
        elif "(" not in line:
            named, obf = line.strip().split(" -> ")
            fields[(current, obf)] = named.split(" ")[1]
    return classes, fields


# Defined by the Entity constructor before defineSynchedData (read from its bytecode in
# both versions). DATA_AIR_SUPPLY_ID is the exporter's per-type getMaxAirSupply().
ENTITY_CONSTRUCTOR_DEFAULTS = {
    "DATA_SHARED_FLAGS_ID": ["byte", 0],
    "DATA_CUSTOM_NAME": ["other", None],
    "DATA_CUSTOM_NAME_VISIBLE": ["bool", False],
    "DATA_SILENT": ["bool", False],
    "DATA_NO_GRAVITY": ["bool", False],
    "DATA_POSE": ["other", None],
    "DATA_TICKS_FROZEN": ["int", 0],
}


CONSTANT = re.compile(
    r"getstatic\s+#\d+\s+// Field (\w+):L[\w$]+;\n"
    r"\s+\d+: (iconst_m1|iconst_(\d)|bipush\s+(-?\d+)|sipush\s+(-?\d+)|ldc2_w\s+#\d+\s+// long (-?\d+)l)\n"
    r"\s+\d+: invokestatic\s+#\d+\s+// Method java/lang/(Byte|Integer|Long|Boolean)\.valueOf"
)


def constant_defines(javap_text):
    """Constant defaults defined by defineSynchedData(Builder), by accessor field."""
    body = javap_text.split("protected void a(ama$a);", 1)[1].split("return\n", 1)[0]
    out = {}
    for field, op, small, bipush, sipush, long_value, boxed in CONSTANT.findall(body):
        value = -1 if op == "iconst_m1" else int(small or bipush or sipush or long_value)
        kind = {"Byte": "byte", "Integer": "int", "Long": "long", "Boolean": "bool"}[boxed]
        out[field] = [kind, bool(value) if kind == "bool" else value]
    return out


def main(export, mapping_path, out_path, minecraft, javap_dir=None):
    classes, fields = mappings(mapping_path)
    parent, data, types, defaults, air, partial = {}, {}, {}, {}, {}, []
    entity_type = next(o for o, n in classes.items() if n == "net.minecraft.world.entity.EntityType")
    for line in open(export, encoding="utf-8"):
        kind, *rest = line.split()
        if kind == "type":
            types["minecraft:" + fields[(entity_type, rest[0])].lower()] = rest[1]
        elif kind == "partial":
            partial.append(rest[0])
        elif kind == "air":
            air[rest[0]] = int(rest[1])
        elif kind == "default":
            owner, index, value = rest
            kind_name, text = value.split(":", 1)
            parsed = {"byte": int, "int": int, "long": int, "float": float, "bool": lambda t: t == "true"}.get(kind_name)
            defaults.setdefault(owner, {})[int(index)] = [kind_name, parsed(text) if parsed else None]
        elif kind == "class":
            parent[rest[0]] = rest[1]
        elif kind == "data":
            owner, field, index = rest
            data.setdefault(owner, {})[fields[(owner, field)]] = int(index)
    for obf in partial:
        path = os.path.join(javap_dir or "", obf + ".txt")
        if not javap_dir or not os.path.exists(path):
            sys.exit(f"{classes[obf]} ({obf}) stopped early; pass its javap in {path}")
        for field, default in constant_defines(open(path, encoding="utf-8").read()).items():
            index = data[obf][fields[(obf, field)]]
            defaults.setdefault(obf, {}).setdefault(index, default)
    short = lambda obf: classes[obf].rsplit(".", 1)[1]
    owners = {}
    for obf in set(parent) | set(data):
        chain, cursor = [], obf
        while cursor in parent or cursor in data:
            if cursor in data:
                chain.append(cursor)
            cursor = parent.get(cursor)
        owners[obf] = chain
    entities = {}
    for name, obf in sorted(types.items()):
        rows = []
        for owner in reversed(owners.get(obf, [])):
            for field, index in sorted(data[owner].items(), key=lambda kv: kv[1]):
                # [kind, value] of the type's default; "other" keeps no value and
                # "unknown" marks fields the exporter could not define (see source json).
                default = defaults.get(obf, {}).get(index)
                if default is None and short(owner) == "Entity":
                    default = ENTITY_CONSTRUCTOR_DEFAULTS.get(field)
                    if field == "DATA_AIR_SUPPLY_ID" and obf in air:
                        default = ["int", air[obf]]
                rows.append([short(owner), field, index, *(default or ["unknown", None])])
        entities[name] = rows
    json.dump({"minecraft": minecraft, "entities": entities}, open(out_path, "w"), indent=0, sort_keys=True)
    open(out_path, "a").write("\n")


if __name__ == "__main__":
    main(*sys.argv[1:])
