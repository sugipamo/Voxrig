#!/usr/bin/env python3
"""Record unchanged original bundled Fraction constructor/arithmetic fields and equality."""
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
    rows=[]
    numbers=[-2147483648,-2147483647,-65536,-2,-1,0,1,2,3,16,64,65536,2147483646,2147483647]
    for n in numbers:
        for d in [-2147483648,-2147483647,-2,-1,0,1,2,16,64,65536,2147483646,2147483647]:
            rows.append({'operation':'from','arguments':[n,d]})
    fields=[(0,1),(0,64),(1,1),(-1,1),(1,16),(1,64),(2,128),(1,2147483647),(-1,2147483647),(2147483647,1),(-2147483648,1),(2147483647,64),(2,2147483646),(3,65536),(65536,3),(-65536,3)]
    for op in ['add','multiply']:
        for a in fields:
            for b in fields:rows.append({'operation':op,'arguments':list(a+b)})
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
    sources=['scripts/ExportFractionConstructors.java']
    named=b/'named-sources';named.mkdir(exist_ok=True)
    for s in sources:(named/Path(s).name).write_text('package voxrig.oracle;\n'+(ROOT/s).read_text())
    compiler=b/'CompileOwnTool.java';compiler.write_text(COMPILER);classes=b/'own-classes';inputs=b/'inputs.json';raw=b/'raw.json';request_bytes=encoded(requests())
    if not a.normalize_only:
        inputs.write_bytes(request_bytes)
        for label,args in [('compile',[str(compiler),cp,str(classes),*(str(named/Path(s).name) for s in sources)]),('run',['-cp',str(classes)+os.pathsep+cp,'voxrig.oracle.ExportFractionConstructors',str(inputs),str(raw)])]:
            with (b/(label+'.log')).open('w') as log:subprocess.run(['java','-Xmx512M','-XX:ActiveProcessorCount=1',*args],cwd=b,stdout=log,stderr=subprocess.STDOUT,check=True)
    assert inputs.read_bytes()==request_bytes
    data=scalar_json(json.loads(raw.read_text()));rules_file='data/client_api/fraction_constructor_rules-1.21.11.json';rules_bytes=encoded({'library':'original bundled org.apache.commons.lang3.math.Fraction','factory':'getFraction(int,int)','operations':['add','multiplyBy'],'comparison':'value.equals;retain returned numerator/denominator representation'});cases_file='data/client_api/fraction_constructor_cases-1.21.11.json.gz';case_bytes=gzip.compress(encoded({'schema':1,'cases':data['cases'],'pairs':data['pairs']},compact=True),mtime=0)
    source=encoded({'schema':1,'authority':'Unchanged original bundled Apache Commons Fraction factories/arithmetic/getter fields/value.equals used by native bundle constructors. This is arithmetic evidence only,not complete item/bundle constructor or registry/cache/admission proof.','original_server_jar_sha1':sha1,'mappings_sha256':msha,'original_classpath_entries_sha256':classpath,'java_version':data['java_version'],'java_executable_sha256':digest(Path(__import__('shutil').which('java')).resolve().read_bytes()),'jdk_modules_sha256':digest((Path(__import__('shutil').which('java')).resolve().parent.parent/'lib/modules').read_bytes()),'generators_sha256':{s:digest((ROOT/s).read_bytes()) for s in sources+['scripts/export_fraction_constructors.py','scripts/export_text_core.py','scripts/export_item_properties.py','scripts/export_regular_clicks.py']},'generated_compiler_sha256':digest(COMPILER.encode()),'requests_sha256':digest(request_bytes),'raw_output_sha256':digest(raw.read_bytes()),'local_original_fraction_constructor_inspection_sha256':digest((b/'original-fraction-bytecode.log').read_bytes()),'files_sha256':{rules_file:digest(rules_bytes),cases_file:digest(case_bytes)},'cases':len(data['cases']),'accepted':sum(c['accepted'] for c in data['cases']),'pairs':len(data['pairs'])})
    for name,content in [(rules_file,rules_bytes),(cases_file,case_bytes),('data/client_api/fraction_constructor_source.json',source)]:
        path=ROOT/name
        if a.check:
            if path.read_bytes()!=content:raise SystemExit('native Fraction constructor facts differ: '+name)
        else:path.write_bytes(content)
    print('Original Fraction constructor facts verified' if a.check else 'Original Fraction constructor facts generated',len(data['cases']),sum(c['accepted'] for c in data['cases']),len(data['pairs']))


if __name__=='__main__':main()
