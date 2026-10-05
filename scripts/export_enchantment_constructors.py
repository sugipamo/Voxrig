#!/usr/bin/env python3
"""Record unchanged native enchantment stream constructors, map fields and equality."""
import argparse
import gzip
import json
import os
import subprocess
from pathlib import Path
from export_regular_clicks import ROOT, VERSIONS, digest, encoded, original_modern_classpath
from export_item_properties import COMPILER
from export_text_core import scalar_json


def requests():
    def varint(value):
        value &= 0xffffffff
        out=bytearray()
        while value > 127:
            out.append((value & 127) | 128)
            value >>= 7
        out.append(value)
        return bytes(out)
    rows=[]
    for component in ['minecraft:enchantments','minecraft:stored_enchantments']:
        def add(name,entries=None,wire=None):
            wire = wire if wire is not None else varint(len(entries))+b''.join(varint(k)+varint(v) for k,v in entries)
            rows.append({'case':component+'-'+name,'component':component,'input_hex':wire.hex()})
        add('empty',[])
        for key in [0,40]:
            for level in [-2147483648,-1000,-1,0,1,2,3,127,128,255,256,257,2147483647]:add(f'level-{key}-{level}',[(key,level)])
        for a,b in [(-1,2),(2,-1),(0,2),(2,0),(2,2),(2,3),(-1,-1),(255,256),(256,255)]:add(f'duplicate-{a}-{b}',[(40,a),(40,b)])
        for name,entries in [('order-a',[(0,1),(40,2)]),('order-b',[(40,2),(0,1)]),('duplicate-order-a',[(40,3),(0,1),(40,2)]),('duplicate-order-b',[(40,2),(0,1)])]:add(name,entries)
        for wire in [b'\x80\x00',b'\x81\x00\xa8\x00\x82\x00',b'\x01\x28',b'\x01',b'\xff\xff\xff\xff\x0f',b'\x00\x00']:add('wire-'+wire.hex(),wire=wire)
    return rows


def main():
    p=argparse.ArgumentParser(description=__doc__)
    for name in ['downloads','modern-classpath-file','runtime-output']:
        p.add_argument('--'+name,type=Path,required=True)
    p.add_argument('--normalize-only',action='store_true');p.add_argument('--check',action='store_true')
    a=p.parse_args();b=a.runtime_output.resolve();b.mkdir(parents=True,exist_ok=True)
    jar=(a.downloads/'1.21.11-server.jar').resolve();mapping=a.downloads/'1.21.11-server-mappings.txt';sha1,msha=VERSIONS['1.21.11']
    if digest(jar.read_bytes(),'sha1')!=sha1 or digest(mapping.read_bytes())!=msha:raise SystemExit('original inputs differ')
    cp=os.pathsep.join(str(Path(s).resolve()) for s in a.modern_classpath_file.read_text().strip().split(os.pathsep));classpath=original_modern_classpath(jar,cp)
    sources=['scripts/ExportInventoryTransfers.java','scripts/ExportItemComponents.java','scripts/ExportItemProperties.java','scripts/ExportItemComponentSchema.java','scripts/ExportComponentValueRules.java','scripts/ExportNbtSemantics.java','scripts/ExportComponentNormalization.java','scripts/ExportTextCore.java','scripts/ExportEnchantmentConstructors.java']
    named=b/'named-sources';named.mkdir(exist_ok=True)
    for s in sources:(named/Path(s).name).write_text('package voxrig.oracle;\n'+(ROOT/s).read_text())
    compiler=b/'CompileOwnTool.java';compiler.write_text(COMPILER);classes=b/'own-classes';inputs=b/'inputs.json';raw=b/'raw.json';request_bytes=encoded(requests())
    if not a.normalize_only:
        inputs.write_bytes(request_bytes)
        for label,args in [('compile',[str(compiler),cp,str(classes),*(str(named/Path(s).name) for s in sources)]),('run',['-cp',str(classes)+os.pathsep+cp,'voxrig.oracle.ExportEnchantmentConstructors',str(inputs),str(raw)])]:
            with (b/(label+'.log')).open('w') as log:subprocess.run(['java','-Xmx512M','-XX:ActiveProcessorCount=1',*args],cwd=b,stdout=log,stderr=subprocess.STDOUT,check=True)
    assert inputs.read_bytes()==request_bytes
    data=scalar_json(json.loads(raw.read_text()));rules_file='data/client_api/enchantment_constructor_rules-1.21.11.json';rules_bytes=encoded({'registry':'minecraft:enchantment','level_minimum':0,'level_maximum':255,'duplicate_keys':'last_wins','schema_forward_node':18,'schema_map_node':19,'schema_key_node':20,'schema_value_node':2});cases_file='data/client_api/enchantment_constructor_cases-1.21.11.json.gz';case_bytes=gzip.compress(encoded({'schema':1,'cases':data['cases'],'pairs':data['pairs']},compact=True),mtime=0)
    source=encoded({'schema':1,'authority':'Unchanged original enchantment component stream codecs, constructor acceptance, effective map entries/original encoded streams and value.equals under observed vanilla registry context; no live registry, persistent/cache or gameplay proof.','original_server_jar_sha1':sha1,'mappings_sha256':msha,'original_classpath_entries_sha256':classpath,'java_version':data['java_version'],'java_executable_sha256':digest(Path(__import__('shutil').which('java')).resolve().read_bytes()),'jdk_modules_sha256':digest((Path(__import__('shutil').which('java')).resolve().parent.parent/'lib/modules').read_bytes()),'generators_sha256':{s:digest((ROOT/s).read_bytes()) for s in sources+['scripts/export_enchantment_constructors.py','scripts/export_text_core.py','scripts/export_item_properties.py','scripts/export_regular_clicks.py']},'generated_compiler_sha256':digest(COMPILER.encode()),'requests_sha256':digest(request_bytes),'raw_output_sha256':digest(raw.read_bytes()),'local_original_enchantment_constructor_inspection_sha256':digest((b/'original-enchantments-bytecode.log').read_bytes()),'files_sha256':{rules_file:digest(rules_bytes),cases_file:digest(case_bytes)},'cases':len(data['cases']),'accepted':sum(c['accepted'] for c in data['cases']),'pairs':len(data['pairs'])})
    for name,content in [(rules_file,rules_bytes),(cases_file,case_bytes),('data/client_api/enchantment_constructor_source.json',source)]:
        path=ROOT/name
        if a.check:
            if path.read_bytes()!=content:raise SystemExit('native enchantment constructor facts differ: '+name)
        else:path.write_bytes(content)
    print('Original enchantment constructor facts verified' if a.check else 'Original enchantment constructor facts generated',len(data['cases']),sum(c['accepted'] for c in data['cases']),len(data['pairs']))


if __name__=='__main__':main()
