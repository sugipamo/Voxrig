#!/usr/bin/env python3
"""Common received column metadata against original official packets/native getters."""
import argparse
import functools
import hashlib
import json
from pathlib import Path
import struct
import subprocess

from run_climbing_control import REPO, run
from run_common_native import NativeSocialReader, until
from run_player_context import unpack_position

KINDS = ["WorldSurfaceWg", "WorldSurface", "OceanFloorWg", "OceanFloor", "MotionBlocking", "MotionBlockingNoLeaves"]
NAMES = ["WORLD_SURFACE_WG", "WORLD_SURFACE", "OCEAN_FLOOR_WG", "OCEAN_FLOOR", "MOTION_BLOCKING", "MOTION_BLOCKING_NO_LEAVES"]
NATIVE_RUNTIME = {}


def reader(raw, version):
    return NativeSocialReader(dict(body_hex=raw.hex()), version)


def nbt(r, named):
    start = r.cursor
    def string():
        return r.take(int.from_bytes(r.take(2), "big")).decode()
    def value(tag, depth=0):
        if depth > 64:
            raise RuntimeError("oracle NBT depth exceeded")
        if tag in (1, 2, 3, 4, 5, 6):
            return struct.unpack({1:">b",2:">h",3:">i",4:">q",5:">f",6:">d"}[tag], r.take({1:1,2:2,3:4,4:8,5:4,6:8}[tag]))[0]
        if tag == 8:
            return string()
        if tag == 10:
            result = {}
            while child := r.take(1)[0]:
                key = string()
                result[key] = (child, value(child, depth+1))
            return result
        if tag in (7, 9, 11, 12):
            child = r.take(1)[0] if tag == 9 else {7:1,11:3,12:4}[tag]
            count = int.from_bytes(r.take(4), "big", signed=True)
            if not 0 <= count <= 2_097_152:
                raise RuntimeError("oracle NBT array budget exceeded")
            return [value(child, depth+1) for _ in range(count)]
        raise RuntimeError("invalid original NBT tag")
    tag = r.take(1)[0]
    if tag and named:
        string()
    data = value(tag) if tag else None
    return data, r.raw[start:r.cursor]


def palette(r, count, old=False):
    bits = r.take(1)[0]
    if bits == 0:
        return [r.integer()] * count
    entries = [r.integer() for _ in range(r.integer())] if bits <= (8 if count == 4096 else 3) else None
    width = 64 // bits
    words = (count+width-1)//width
    if old and r.integer() != words:
        raise RuntimeError("invalid original old palette array length")
    result = []
    for _ in range(words):
        word = int.from_bytes(r.take(8), "big")
        for index in range(width):
            if len(result) == count:
                break
            cell = (word >> (index*bits)) & ((1<<bits)-1)
            result.append(entries[cell] if entries is not None else cell)
    return result


def decode(version, packet, minimum, height):
    r = NativeSocialReader(packet, version)
    old = version == "1.16.1"
    kind = packet["packet_id"]
    result = dict(biomes={}, maps={}, entities={})
    if kind == (0x21 if old else 0x2c):
        column = tuple(struct.unpack(">ii", r.take(8)))
        maps = []
        if old:
            full = r.boolean()
            r.boolean()
            r.integer()
            fields, raw = nbt(r, True)
            if fields:
                for name, tag in fields.items():
                    if name in NAMES and tag[0] == 12:
                        maps.append(dict(kind=KINDS[NAMES.index(name)], min_y=0, height=256, words=tag[1]))
            if full:
                result["biomes"][column] = list(struct.unpack(">1024i", r.take(4096)))
            r.take(r.integer())
            for _ in range(r.integer()):
                fields, encoded = nbt(r, True)
                pos = tuple(fields[key][1] for key in ("x","y","z"))
                result["entities"][pos] = dict(kind={"LegacyIdentifier":fields.get("id",(8,None))[1]}, encoded_nbt=list(encoded))
        else:
            for _ in range(r.integer()):
                key = r.integer()
                words = [struct.unpack(">q",r.take(8))[0] for _ in range(r.integer())]
                maps.append(dict(kind=KINDS[key], min_y=minimum, height=height, words=words))
            data = reader(r.take(r.integer()), version)
            biomes = []
            for _ in range(height//16):
                data.take(2)
                palette(data,4096)
                biomes.extend(palette(data,64))
            data.end()
            result["biomes"][column] = biomes
            for _ in range(r.integer()):
                xz = r.take(1)[0]
                y = int.from_bytes(r.take(2),"big",signed=True)
                native_kind = r.integer()
                _, encoded = nbt(r, False)
                pos = (column[0]*16+(xz>>4),y,column[1]*16+(xz&15))
                result["entities"][pos] = dict(kind={"ModernType":native_kind},encoded_nbt=list(encoded))
            # Light is outside this metadata oracle. Its original frame remains
            # intact and the existing light tests own that decoder.
            r.take(len(r.raw)-r.cursor)
        result["maps"][column] = sorted({m["kind"]:m for m in maps}.values(),key=lambda m:KINDS.index(m["kind"]))
        result["column"] = column
    elif kind == (0x09 if old else 0x06):
        pos = tuple(unpack_position(r.take(8)))
        native_kind = r.take(1)[0] if old else r.integer()
        _, encoded = nbt(r, old)
        result["entities"][pos] = dict(kind={"LegacyUpdateAction" if old else "ModernType":native_kind},encoded_nbt=list(encoded))
    elif not old and kind == 0x0d:
        for _ in range(r.integer()):
            packed = int.from_bytes(r.take(8),"big")
            def signed(value):
                return value-(1<<32) if value&(1<<31) else value
            column = (signed(packed&0xffffffff),signed(packed>>32))
            data = reader(r.take(r.integer()),version)
            ids = []
            for _ in range(height//16):
                ids.extend(palette(data,64))
            data.end()
            result["biomes"][column] = ids
    else:
        raise RuntimeError("receipt points to a different original packet")
    r.end()
    return result


def verify(version, context, trace):
    peers = [p for p in trace.since(0) if p["direction"]=="clientbound" and p["phase"] in ("configuration","play")]
    column = tuple(context["chunk"]["position"])
    minimum, height = (0,256) if version=="1.16.1" else (context["biomes"]["value"]["min_y"],context["biomes"]["value"]["height"])
    originals = []
    decoded = {}
    def original(sequence):
        if not 1 <= sequence or not context["chunk"]["session"]["world_generation"] <= sequence <= context["receive_sequence"]:
            raise RuntimeError("chunk receipt crossed world/capture boundary")
        packet = peers[sequence-1]
        if packet["phase"] != "play":
            raise RuntimeError("chunk receipt points to configuration")
        if sequence not in decoded:
            decoded[sequence] = decode(version,packet,minimum,height)
            originals.append(dict(sequence=sequence,packet=packet))
        return decoded[sequence]
    if original(context["chunk"]["load_sequence"]).get("column") != column:
        raise RuntimeError("column incarnation points to another original load")
    for name, key in (("biomes","biomes"),("heightmaps","maps")):
        field = context[name]
        if field is None:
            continue
        if field["source"]["kind"] != "received":
            raise RuntimeError("chunk metadata supplied model authority")
        expected = original(field["source"]["sequence"])[key][column]
        actual = field["value"]["ids"] if name=="biomes" else field["value"]
        if actual != expected:
            raise RuntimeError(name+" differs from original payload")
    for field in context["block_entities"]:
        if field["source"]["kind"] != "received":
            raise RuntimeError("entity supplied a model source")
        value = field["value"]
        expected = original(field["source"]["sequence"])["entities"][tuple(value["position"])]
        if value["kind"] != expected["kind"] or value["encoded_nbt"] != expected["encoded_nbt"]:
            raise RuntimeError("entity differs from original kind/NBT bytes")
    return originals


def prepare_native(version, jars):
    from movement_oracle.run import classpath, VERSIONS, SPECIAL_SOURCE_SHA256
    root = REPO/".local/chunk-context/oracle"/version
    root.mkdir(parents=True,exist_ok=True)
    named = root/"named.jar"
    jar = jars/(version+"-server.jar")
    mappings = jars/(version+"-server-mappings.txt")
    tool = jars/"SpecialSource-1.11.4-shaded.jar"
    sha = lambda path:hashlib.sha256(path.read_bytes()).hexdigest()
    inputs = dict(server_sha1=hashlib.sha1(jar.read_bytes()).hexdigest(),
                  mappings_sha1=hashlib.sha1(mappings.read_bytes()).hexdigest(),special_source_sha256=sha(tool))
    assert inputs["server_sha1"]==VERSIONS[version][0]
    assert inputs["mappings_sha1"]==VERSIONS[version][1]
    assert inputs["special_source_sha256"]==SPECIAL_SOURCE_SHA256
    game, libraries = classpath(version,jar,root)
    marker = root/"heightmap-inputs.json"
    previous = json.loads(marker.read_text()) if marker.exists() else {}
    if previous.get("inputs") != inputs or not named.exists() or previous.get("named_jar_sha256") != sha(named):
        with (root/"remap.log").open("w") as log:
            subprocess.run(["java","-Xmx512m","-jar",str(tool),"--in-jar",str(game),"--out-jar",str(named),"--srg-in",str(mappings)],check=True,stdout=log,stderr=log)
        marker.write_text(json.dumps(dict(inputs=inputs,named_jar_sha256=sha(named))))
    classes = root/"heightmap-classes"
    classes.mkdir(exist_ok=True)
    subprocess.run(["javac","--release","21","-d",str(classes),str(REPO/"scripts/VerifyReceivedHeightmap.java")],check=True,capture_output=True)
    cp = ":".join(map(str,[classes,named]+libraries[1:]))
    NATIVE_RUNTIME[version] = cp
    return dict(**inputs,named_jar_sha256=sha(named),harness_sha256=sha(REPO/"scripts/VerifyReceivedHeightmap.java"),
                scope="original mapped BitStorage constructor/get; unchanged official game server")


def native_heights(version, maps, samples):
    cp = NATIVE_RUNTIME[version]
    rows = [" ".join(map(str,[m["height"],m["min_y"]]+m["words"])) for m in maps]
    output = subprocess.run(["java","-cp",cp,"VerifyReceivedHeightmap",version],input="\n".join(rows)+"\n",text=True,capture_output=True,check=True)
    decoded = [json.loads(line) for line in output.stdout.splitlines()]
    if len(decoded) != len(samples):
        raise RuntimeError("native heightmap result count differs")
    results = []
    for m,sample,values in zip(maps,samples,decoded):
        if m["kind"] != sample["kind"] or values != sample["values"] or values[17] != sample["first_available_y"]:
            raise RuntimeError("SDK heightmap sample differs from original native storage")
        results.append(dict(kind=m["kind"],values=values))
    return results


def native_biome_id(trace, name):
    for packet in trace.since(0):
        if packet["direction"]=="clientbound" and packet["phase"]=="configuration" and packet["packet_id"]==7:
            r = NativeSocialReader(packet,"1.21.11")
            registry = r.string()
            found = None
            for index in range(r.integer()):
                entry = r.string()
                if r.boolean():
                    r.nbt(r.take(1)[0])
                if registry=="minecraft:worldgen/biome" and entry==name:
                    found = index
            r.end()
            if found is not None:
                return dict(id=found,original=packet)
    raise RuntimeError("native received biome registry entry missing")


def check(version, command, request, trace, report, *, sdk):
    report["sdk"] = sdk
    def capture(name,save=False):
        response = request("chunk_context",position=[0,0],save=save)
        context = response["context"]
        if context is None:
            raise RuntimeError("expected loaded column context missing")
        item = dict(name=name,context=context,originals=verify(version,context,trace))
        if context["heightmaps"] is not None and context["heightmaps"]["value"]:
            item["native_storage"] = native_heights(version,context["heightmaps"]["value"],response["heightmap_samples"])
        report["checks"].append(item)
        return context
    def reload():
        command("tp ClimbingProbe 400.5 65 0.5 0 0")
        until(lambda: request("chunk_context",position=[0,0])["context"] is None,20)
        command("tp ClimbingProbe 0.5 65 -0.5 0 0")
        return until(lambda: (value if (value:=request("chunk_context",position=[0,0]))["context"] is not None
                              and value["context"]["heightmaps"] is not None else None),20)
    def sign(marker):
        fields = ("{Text1:'{\"text\":\""+marker+"\"}'}") if version=="1.16.1" else (
            "{front_text:{messages:[\""+marker+"\",\"\",\"\",\"\"],color:\"black\",has_glowing_text:0b}}")
        reply = command("data merge block 1 65 1 "+fields)
        if "Modified block data" not in reply:
            raise RuntimeError("native sign update failed: "+reply)
        return command("data get block 1 65 1")
    initial = capture("initial_received_column")
    command("setblock 1 65 1 minecraft:oak_sign[rotation=0]")
    sign("column-first")
    reload()
    first = capture("full_column_sign_and_heightmaps",save=True)
    sign_entry = next((v for v in first["block_entities"] if v["value"]["position"]==[1,65,1]),None)
    if sign_entry is None or not first["block_entities_complete"]:
        raise RuntimeError("full column lost its sign entity/list coverage")
    report["checks"][-1]["native_sign"] = command("data get block 1 65 1")
    saved = request("saved_chunk_context")
    if saved != first:
        raise RuntimeError("saved immutable capture differs immediately")
    if version=="1.21.11":
        desert = native_biome_id(trace,"minecraft:desert")
        command("fillbiome 0 64 0 15 79 15 minecraft:desert")
        def changed_biome():
            value = request("chunk_context",position=[0,0])["context"]
            biome = value["biomes"]
            index = ((65-biome["value"]["min_y"])//4)*16
            return value if biome["value"]["ids"][index]==desert["id"] and biome["source"]["sequence"]>first["receive_sequence"] else None
        until(changed_biome,10)
        changed = capture("dedicated_biome_update")
        if changed["chunk"]!=first["chunk"] or changed["heightmaps"]!=first["heightmaps"] or changed["block_entities"]!=first["block_entities"]:
            raise RuntimeError("biome-only update replaced unrelated receipts/incarnation")
        report["checks"][-1]["native_biome"] = dict(registry=desert,result=command("execute if biome 1 65 1 minecraft:desert"))
        if "Test passed" not in report["checks"][-1]["native_biome"]["result"]:
            raise RuntimeError("native desert biome condition failed")
    else:
        report["checks"].append(dict(name="legacy_dedicated_biome_update_unavailable",provided=False))
    before = request("chunk_context",position=[0,0])["context"]
    native = sign("column-second")
    def fresh_sign():
        value = request("chunk_context",position=[0,0])["context"]
        return value if any(v["value"]["position"]==[1,65,1] and v["source"]["sequence"]>before["receive_sequence"] for v in value["block_entities"]) else None
    until(fresh_sign,10)
    update = capture("dedicated_entity_update")
    report["checks"][-1]["native_sign"] = native
    if update["chunk"]!=first["chunk"] or update["biomes"]!=before["biomes"] or update["heightmaps"]!=before["heightmaps"]:
        raise RuntimeError("entity update refreshed unrelated receipts")
    command("setblock 1 65 1 minecraft:air")
    until(lambda: (value if (value:=request("chunk_context",position=[0,0])["context"])["heightmaps"] is None
                   and not any(v["value"]["position"]==[1,65,1] for v in value["block_entities"]) else None),10)
    invalid = capture("terrain_invalidates_heightmap_and_entity")
    if invalid["block_entities_complete"]:
        raise RuntimeError("changed column still claims an untouched full entity list")
    if request("saved_chunk_context") != saved:
        raise RuntimeError("terrain update modified old immutable capture")
    report["checks"].append(dict(name="saved_capture_keeps_original_incarnation",context=saved,originals=verify(version,saved,trace)))
    reload()
    renewed = capture("unload_reload_new_incarnation")
    if renewed["chunk"] == first["chunk"] or any(v["value"]["position"]==[1,65,1] for v in renewed["block_entities"]):
        raise RuntimeError("reloaded column reused old ownership or removed entity")
    old_generation = renewed["chunk"]["session"]["world_generation"]
    command("execute in minecraft:the_nether run tp ClimbingProbe 0.5 80 0.5 0 0")
    until(lambda: (value if (value:=request("chunk_context",position=[0,0])["context"]) is not None
                   and value["chunk"]["session"]["world_generation"]!=old_generation else None),20)
    nether = capture("new_world_owned_column")
    if request("saved_chunk_context") != saved:
        raise RuntimeError("world replacement reinterpreted old saved column")
    trace.expect_disconnect()
    closed = request("chunk_context_disconnect",position=[0,0])
    if closed["context"]["chunk"]!=nether["chunk"] or closed["saved"]!=saved:
        raise RuntimeError("closed read lost column ownership/saved capture")
    report["checks"].append(dict(name="read_after_close",context=closed["context"],saved=closed["saved"],originals=verify(version,closed["context"],trace)))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--accept-eula",action="store_true",required=True)
    parser.add_argument("--version",choices=("1.16.1","1.21.11"),action="append")
    parser.add_argument("--binary",type=Path,required=True)
    parser.add_argument("--jars",type=Path,default=REPO/".local/climbing/downloads")
    parser.add_argument("--compiled-sdk-revision",required=True)
    args = parser.parse_args()
    revision = subprocess.check_output(["git","rev-parse","HEAD"],cwd=REPO,text=True).strip()
    compiled = subprocess.check_output(["git","rev-parse",args.compiled_sdk_revision],cwd=REPO,text=True).strip()
    if subprocess.check_output(["git","diff",compiled,"HEAD","--","src","examples","Cargo.toml","Cargo.lock"],cwd=REPO):
        raise RuntimeError("declared compiled SDK differs from current runtime source")
    sha = lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
    sdk = dict(source_revision=revision,binary_build_revision=compiled,
               source_diff_sha256=hashlib.sha256(subprocess.check_output(["git","diff","HEAD"],cwd=REPO)).hexdigest(),
               binary_sha256=sha(args.binary),artifacts={p:sha(REPO/p) for p in (
                   "Cargo.lock","scripts/run_chunk_context.py","scripts/VerifyReceivedHeightmap.java",
                   "scripts/run_climbing_control.py","scripts/run_common_native.py","examples/climbing_control_probe.rs")})
    for version in args.version or ("1.16.1","1.21.11"):
        sdk["native_heightmap_oracle"] = prepare_native(version,args.jars.resolve())
        run(version,args.binary.resolve(),args.jars.resolve(),check=functools.partial(check,sdk=sdk.copy()),
            server_properties={"view-distance":3,"simulation-distance":2,"max-players":5})


if __name__=="__main__":
    main()
