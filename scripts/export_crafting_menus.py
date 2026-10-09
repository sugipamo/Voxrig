#!/usr/bin/env python3
"""Inspect native crafting menu topology and static input slot rules in both versions.

Official original constructors and slot methods only; no recipe or result-take
algorithm is copied or replaced. JVMs run sequentially with 512 MiB/one CPU.
"""
import argparse
import gzip
import json
import os
import subprocess
from pathlib import Path
from export_regular_clicks import ROOT, VERSIONS, digest, encoded, original_modern_classpath
from export_item_properties import COMPILER


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ['downloads','modern-classpath-file','runtime-output']:
        parser.add_argument('--'+name,type=Path,required=True)
    parser.add_argument('--normalize-only',action='store_true')
    parser.add_argument('--check',action='store_true')
    args=parser.parse_args();base=args.runtime_output.resolve();base.mkdir(parents=True,exist_ok=True)
    cp=os.pathsep.join(str(Path(p).resolve()) for p in args.modern_classpath_file.read_text().strip().split(os.pathsep))
    sources=['scripts/'+name+'.java' for name in ['ExportInventoryTransfers','ExportCraftingMenus']]
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
        if not args.normalize_only:
            with (base/(version+'-run.log')).open('w') as log:
                subprocess.run(['java','-Xmx512M','-XX:ActiveProcessorCount=1','-cp',str(classes)+os.pathsep+runtime_cp,'voxrig.oracle.ExportCraftingMenus',version,str(raw)],cwd=base,stdout=log,stderr=subprocess.STDOUT,check=True)
        data=json.loads(raw.read_text());assert data['version']==version and len(data['menus'])==2
        path='data/client_api/crafting_menus-'+version+'.json.gz';outputs[path]=gzip.compress(encoded(data,compact=True),mtime=0)
        runs.append({'version':version,'original_server_jar_sha1':jar_sha,'mappings_sha256':mapping_sha,'original_classpath_entries_sha256':classpath,'raw_output_sha256':digest(raw.read_bytes()),'files_sha256':{path:digest(outputs[path])}})
    outputs['data/client_api/crafting_menu_source.json']=encoded({'schema':1,'authority':'Unmodified native player and crafting-table constructors, actual grid width/height/container identity/result subclass and player slot indices; actual empty-slot mayPickup, capacity and mayPlace for all default items.','scope':'Topology and static input rules only; no recipe matching, crafting result take, input consumption, network, modes or lifecycle proof.','generators_sha256':{p:digest((ROOT/p).read_bytes()) for p in sources+['scripts/export_crafting_menus.py']},'generated_compiler_sha256':digest(COMPILER.encode()),'runs':runs})
    for name,data in outputs.items():
        target=ROOT/name
        if args.check:assert target.read_bytes()==data,name
        else:target.write_bytes(data)
    print('original crafting menu topology/rules verified' if args.check else 'original crafting menu topology/rules generated',[(r['version'],2) for r in runs])

if __name__=='__main__':main()
