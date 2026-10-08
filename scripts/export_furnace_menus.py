#!/usr/bin/env python3
"""Inspect native furnace topology, vanilla fuel slot rules and static outlines.

Official original constructors and slot methods only; no recipe or result-take
algorithm is copied or replaced. JVMs run sequentially with 512 MiB/one CPU.
"""
import argparse
import gzip
import json
import os
import subprocess
import struct
import zipfile
from pathlib import Path
from export_regular_clicks import ROOT, VERSIONS, digest, encoded, original_modern_classpath
from export_item_properties import COMPILER


def sibling_classes(mapping):
    classes={}
    for line in mapping.read_text().splitlines():
        for name in ['BlastFurnaceMenu','SmokerMenu']:
            if line.startswith('net.minecraft.world.inventory.'+name+' -> '):
                classes[name]=line.split(' -> ')[1][:-1]
    return [classes['BlastFurnaceMenu'],classes['SmokerMenu']]


def native_fuel_tag_fields(jar, version):
    # Read factual Fieldref metadata, not Minecraft method bodies/algorithms.
    old = version == '1.16.1'
    if old:
        with zipfile.ZipFile(jar) as z: data=z.read('cdb.class')
    else:
        with zipfile.ZipFile(jar) as z:
            entry=z.read('META-INF/versions.list').decode().strip().split('\t')[2]
            import io
            with zipfile.ZipFile(io.BytesIO(z.read('META-INF/versions/'+entry))) as inner:
                data=inner.read('emb.class')
    assert data[:4] == bytes.fromhex('cafebabe')
    cp=[None];offset=10;count=struct.unpack_from('>H',data,8)[0]
    while len(cp)<count:
        tag=data[offset];offset+=1
        if tag==1:
            size=struct.unpack_from('>H',data,offset)[0];offset+=2
            value=data[offset:offset+size].decode('utf-8', errors='replace');offset+=size
        elif tag in (7,8,16,19,20):
            value=struct.unpack_from('>H',data,offset)[0];offset+=2
        elif tag in (9,10,11,12,17,18):
            value=struct.unpack_from('>HH',data,offset);offset+=4
        elif tag in (3,4): value=None;offset+=4
        elif tag in (5,6): cp.append((tag,None));value=None;offset+=8
        elif tag==15: value=None;offset+=3
        else: raise ValueError('unknown native constant-pool tag '+str(tag))
        cp.append((tag,value))
    fields=[]
    for tag,value in cp[1:]:
        if tag != 9: continue
        cls,name_type=value
        if cp[cp[cls][1]][1] == ('ada' if old else 'bdy'):
            fields.append(cp[cp[name_type][1][0]][1])
    assert fields
    return ','.join(sorted(set(fields))), digest(data)


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ['downloads','modern-classpath-file','runtime-output']:
        parser.add_argument('--'+name,type=Path,required=True)
    parser.add_argument('--normalize-only',action='store_true')
    parser.add_argument('--check',action='store_true')
    args=parser.parse_args();base=args.runtime_output.resolve();base.mkdir(parents=True,exist_ok=True)
    cp=os.pathsep.join(str(Path(p).resolve()) for p in args.modern_classpath_file.read_text().strip().split(os.pathsep))
    sources=['scripts/'+name+'.java' for name in ['ExportInventoryTransfers','ExportItemComponents','ExportStorageOutlines','ExportCraftingOutlines','ExportFurnaceMenus']]
    named=base/'named-sources';named.mkdir(exist_ok=True)
    for path in sources:(named/Path(path).name).write_text('package voxrig.oracle;\n'+(ROOT/path).read_text())
    compiler=base/'CompileOwnTool.java';compiler.write_text(COMPILER);classes=base/'own-classes'
    if not args.normalize_only:
        with (base/'compile.log').open('w') as log:
            subprocess.run(['java','-Xmx512M','-XX:ActiveProcessorCount=1',str(compiler),cp,str(classes),*(str(named/Path(p).name) for p in sources)],cwd=base,stdout=log,stderr=subprocess.STDOUT,check=True)
    outputs={};runs=[]
    for version,(jar_sha,mapping_sha) in VERSIONS.items():
        jar=(args.downloads/(version+'-server.jar')).resolve();mapping=args.downloads/(version+'-server-mappings.txt')
        assert digest(jar.read_bytes(),'sha1')==jar_sha and digest(mapping.read_bytes())==mapping_sha
        runtime_cp=str(jar) if version=='1.16.1' else cp
        classpath={digest(jar.read_bytes()):jar.name} if version=='1.16.1' else original_modern_classpath(jar,cp)
        raw=base/(version+'-raw.json')
        tag_fields, fuel_class_sha256=native_fuel_tag_fields(jar,version)
        if not args.normalize_only:
            with (base/(version+'-run.log')).open('w') as log:
                subprocess.run(['java','-Xmx512M','-XX:ActiveProcessorCount=1','-cp',str(classes)+os.pathsep+runtime_cp,'voxrig.oracle.ExportFurnaceMenus',version,str(raw),str(jar),*sibling_classes(mapping),tag_fields],cwd=base,stdout=log,stderr=subprocess.STDOUT,check=True)
        data=json.loads(raw.read_text());assert data['version']==version and len(data['menus'])==3 and len(data['states'])==24 and len(data['rays'])==144
        path='data/client_api/furnace_menus-'+version+'.json.gz';outputs[path]=gzip.compress(encoded(data,compact=True),mtime=0)
        runs.append({'version':version,'original_server_jar_sha1':jar_sha,'mappings_sha256':mapping_sha,'original_classpath_entries_sha256':classpath,'raw_output_sha256':digest(raw.read_bytes()),'native_fuel_class_sha256':fuel_class_sha256,'native_fuel_tag_fields':tag_fields.split(','),'files_sha256':{path:digest(outputs[path])}})
    outputs['data/client_api/furnace_menu_source.json']=encoded({'schema':1,'authority':'Unmodified native furnace/blast-furnace/smoker constructors, container indices and appended player slots; actual empty-slot mayPickup, default-item mayPlace/capacities (including bucket override); native vanilla tag loading and FuelValues; native outlines and clips.','scope':'Topology and vanilla/default slot rules only. No recipe matching/smelting/ticks, output-take callbacks/XP, network/modes/lifecycle or custom datapack semantics. Skeletal world exposes actual native vanilla FuelValues and an empty recipe manager only for unused construction-time property lookup. Legacy native TagCollection resolves actual JAR JSON builders; modern original vanilla registry/tag loader is reused.','generators_sha256':{p:digest((ROOT/p).read_bytes()) for p in sources+['scripts/export_furnace_menus.py']},'generated_compiler_sha256':digest(COMPILER.encode()),'runs':runs})
    for name,data in outputs.items():
        target=ROOT/name
        if args.check:assert target.read_bytes()==data,name
        else:target.write_bytes(data)
    print('original furnace topology/rules verified' if args.check else 'original furnace topology/rules generated',[(r['version'],3,24,144) for r in runs])

if __name__=='__main__':main()
