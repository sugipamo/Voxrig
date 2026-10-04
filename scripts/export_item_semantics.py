#!/usr/bin/env python3
"""Pin original full-item/value equality, typed persistent inputs and native hash codecs.
Only own wrappers compile; original game classes and methods remain unchanged.
Run one JVM at a time, 512 MiB and CPU 1. Raw runtime outputs stay local.
"""
import argparse
import gzip
import json
import os
from pathlib import Path
import subprocess
from export_regular_clicks import ROOT, VERSIONS, digest, encoded, original_modern_classpath
from export_item_properties import COMPILER

INPUTS = ['data/client_api/item_properties-1.16.1.json',
          'data/client_api/item_properties-1.21.11.json',
          'data/client_api/item_component_cases-1.21.11.json',
          'data/client_api/nbt_semantics-1.21.11.json']

def varint(value):
    value &= 0xffffffff
    result = bytearray()
    while True:
        byte = value & 127; value >>= 7
        result.append(byte | (128 if value else 0))
        if not value: return bytes(result)

def requests(version):
    properties = json.loads((ROOT / f'data/client_api/item_properties-{version}.json').read_text())
    items = [{'case':'default-'+row['name'], 'input_hex':row['encoded_item_hex']} for row in properties['defaults']]
    items += [{'case':'property-'+row['case'], 'input_hex':row['input_item_hex']} for row in properties['cases']]
    prototypes, components = [], []
    if version == '1.21.11':
        samples = json.loads((ROOT / INPUTS[2]).read_text())['samples']
        types = {row['name']:row['native_id'] for row in samples}
        stone = next(row for row in properties['defaults'] if row['name']=='minecraft:stone')['native_id']
        def component(case, name, value, fresh=False):
            field = varint(types[name]) + bytes.fromhex(value)
            item = varint(1) + varint(stone) + b'\x01\x00' + field
            components.append({'case':case,'component':name,'input_hex':value,'item_hex':item.hex(),**({'fresh_hash':True} if fresh else {})})
            items.append({'case':'component-'+case,'input_hex':item.hex()})
        for i, row in enumerate(samples): component('sample-'+str(i), row['name'], row['value_hex'])
        for i, row in enumerate(properties['prototype_values']): component('prototype-value-'+str(i), row['name'], row['value_hex'])
        for row in json.loads((ROOT / INPUTS[3]).read_text())['values']:
            component('nbt-'+row['name'], 'minecraft:custom_data', row['input_hex'], fresh=True)
        # Alternate trusted stream scalar/enum forms; native codecs select canonical values.
        for name in ['minecraft:max_stack_size','minecraft:damage','minecraft:rarity']:
            for value in [0,1,16,127,-1,-2147483648,2147483647]:
                wire=varint(value)
                component('scalar-'+name+'-'+str(value),name,wire.hex())
                if len(wire)<5: component('overlong-'+name+'-'+str(value),name,(wire[:-1]+bytes([wire[-1]|128,0])).hex())
        for row in properties['defaults']:
            if row['represents_empty']: continue
            base=row['encoded_item_hex']
            for index in row['prototype_values']:
                value=properties['prototype_values'][index]
                changed=varint(1)+varint(row['native_id'])+b'\x01\x00'+varint(value['native_id'])+bytes.fromhex(value['value_hex'])
                prototypes.append({'case':row['name']+'-same-prototype-'+str(index),'base_hex':base,'changed_hex':changed.hex()})
            present={properties['prototype_values'][i]['native_id'] for i in row['prototype_values']}
            for name in ['minecraft:custom_data','minecraft:unbreakable','minecraft:damage','minecraft:max_damage']:
                if types[name] not in present:
                    changed=varint(1)+varint(row['native_id'])+b'\x00\x01'+varint(types[name])
                    prototypes.append({'case':row['name']+'-remove-absent-'+name,'base_hex':base,'changed_hex':changed.hex()})
    else:
        # Null and empty legacy tags are compared through actual native item decoding.
        for row in properties['cases']:
            base = next(d['encoded_item_hex'] for d in properties['defaults'] if d['name']==row['item'])
            prototypes.append({'case':row['case'],'base_hex':base,'changed_hex':row['input_item_hex']})
    counted=[]
    for row in items:
        wire=bytes.fromhex(row['input_hex'])
        if wire==b'\x00':continue
        if version=='1.21.11': changed=b'\x02'+wire[1:]
        else:
            index=1
            while wire[index]&128:index+=1
            index+=1;changed=wire[:index]+b'\x02'+wire[index+1:]
        counted.append({'case':row['case']+'-count2','input_hex':changed.hex()})
    return {'version':version,'items':items+counted,'components':components,'prototype_cases':prototypes}

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--downloads',type=Path,required=True)
    parser.add_argument('--modern-classpath-file',type=Path,required=True)
    parser.add_argument('--runtime-output',type=Path,required=True)
    parser.add_argument('--normalize-only',action='store_true')
    parser.add_argument('--check',action='store_true')
    args=parser.parse_args();base=args.runtime_output.resolve();base.mkdir(parents=True,exist_ok=True)
    cp=os.pathsep.join(str(Path(p).resolve()) for p in args.modern_classpath_file.read_text().strip().split(os.pathsep))
    sources=['scripts/ExportInventoryTransfers.java','scripts/ExportItemComponents.java','scripts/ExportItemProperties.java','scripts/RecordingNativeHashOps.java','scripts/ExportItemSemantics.java']
    named=base/'named-sources';named.mkdir(exist_ok=True)
    for source in sources:(named/Path(source).name).write_text('package voxrig.oracle;\n'+(ROOT/source).read_text())
    compiler=base/'CompileOwnTool.java';compiler.write_text(COMPILER);classes=base/'own-classes'
    native_inputs=[]
    for version,(jar_sha1,mapping_sha256) in VERSIONS.items():
        jar=(args.downloads/(version+'-server.jar')).resolve();mapping=args.downloads/(version+'-server-mappings.txt')
        if digest(jar.read_bytes(),'sha1')!=jar_sha1 or digest(mapping.read_bytes())!=mapping_sha256:raise SystemExit('original inputs differ: '+version)
        hashes=original_modern_classpath(jar,cp) if version=='1.21.11' else {digest(jar.read_bytes()):jar.name}
        native_inputs.append((version,jar,jar_sha1,mapping_sha256,hashes))
    if not args.normalize_only:
        with (base/'compile.log').open('w') as log:
            subprocess.run(['java','-Xmx512M','-XX:ActiveProcessorCount=1',str(compiler),cp,str(classes),*(str(named/Path(p).name) for p in sources)],cwd=base,stdout=log,stderr=subprocess.STDOUT,check=True)
    outputs,runs={},[]
    for version,jar,jar_sha1,mapping_sha256,hashes in native_inputs:
        requested=encoded(requests(version),compact=True);request=base/(version+'-requests.json')
        if not args.normalize_only:request.write_bytes(requested)
        elif request.read_bytes()!=requested:raise SystemExit('saved own requests differ: '+version)
        raw=base/(version+'-raw.json')
        if not args.normalize_only:
            with (base/(version+'-run.log')).open('w') as log:
                subprocess.run(['java','-Xmx512M','-XX:ActiveProcessorCount=1','-cp',str(classes)+os.pathsep+(str(jar) if version=='1.16.1' else cp),'voxrig.oracle.ExportItemSemantics',version,str(request),str(raw)],cwd=base,stdout=log,stderr=subprocess.STDOUT,check=True)
        data=json.loads(raw.read_text());assert data['version']==version
        file='data/client_api/item_semantics-'+version+'.json.gz';outputs[file]=gzip.compress(encoded(data,compact=True),mtime=0)
        runs.append({'version':version,'original_server_jar_sha1':jar_sha1,'mappings_sha256':mapping_sha256,'original_classpath_entries_sha256':hashes,'own_requests_sha256':digest(requested),'raw_output_sha256':digest(raw.read_bytes()),'item_rows':len(data['items']),'item_equivalence_groups':data['item_equivalence_groups'],'prototype_cases':len(data['prototype_cases']),'component_rows':len(data.get('components',[])),'hash_nodes':len(data.get('hash_nodes',[])),'files_sha256':{file:digest(outputs[file])}})
    outputs['data/client_api/item_semantics_source.json']=encoded({'schema':1,'authority':'Original independently decoded item/component equality, original effective prototype/patch behavior, unchanged typed persistent encoders with original RegistryOps/HashOps and original hashed-stack creator/codec.','scope':'Standalone primitive/codec oracle. Own observation wrapper delegates each hash factory/builder to unchanged native HashOps and compares outputs to unwrapped native encoders. Own supplied HashGenerator composes original typed encoding and original Guava native-key cache at mapped capacity256; this is not a live ServerPlayer synchronizer/cache proof or gameplay/slot permission. Equality groups are native object comparisons, never hash-based identity claims.','generators_sha256':{p:digest((ROOT/p).read_bytes()) for p in sources+INPUTS+['scripts/export_item_semantics.py','scripts/export_item_properties.py','scripts/export_regular_clicks.py']},'generated_compiler_sha256':digest(COMPILER.encode()),'runs':runs})
    for name,data in outputs.items():
        path=ROOT/name
        if args.check:
            if path.read_bytes()!=data:raise SystemExit('native semantic facts differ: '+name)
        else:path.write_bytes(data)
    print('Original item semantics verified' if args.check else 'Original item semantics generated')
    for run in runs:print(run['version'],'items/groups/prototype-cases/components/hash-nodes',run['item_rows'],run['item_equivalence_groups'],run['prototype_cases'],run['component_rows'],run['hash_nodes'])

if __name__=='__main__':main()
