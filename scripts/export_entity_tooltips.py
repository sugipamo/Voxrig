#!/usr/bin/env python3
"""Record unchanged native Click constructors, chat grammar and equality."""
import argparse
import gzip
import json
import os
import subprocess
import struct
from pathlib import Path
from export_regular_clicks import ROOT, VERSIONS, digest, encoded, original_modern_classpath
from export_item_properties import COMPILER
from export_text_core import tag, scalar_json


def requests():
    import struct
    rows=[]
    base_uuid=(11,struct.pack('>iiiii',4,1,2,3,4))
    def add(name,fields):
        kind,payload=tag(fields);rows.append({'case':name,'input_hex':(bytes([kind])+payload).hex()})
    def entity(id='pig',uuid=base_uuid,name=None):
        result={'action':'show_entity','id':id,'uuid':uuid}
        if name is not None:result['name']=name
        return result
    uuids=[base_uuid,[1,2,3,4],(7,struct.pack('>i',4)+bytes([1,2,3,4])),(12,struct.pack('>iqqqq',4,1,2,3,4)),[1.,2.,3.,4.],[float('nan'),float('inf'),float('-inf'),-0.],(11,struct.pack('>iiii',3,1,2,3)),(11,struct.pack('>iiiiii',5,1,2,3,4,5)),(9,b"\0\0\0\0\0"),1,{},'00000001-0000-0002-0000-000300000004','00000001000000020000000300000004','1-2-3-4-5','+1-+2-+3-+4-+5','１-２-３-４-５','00000001-0000-0002-0000-000300000004tail','invalid','', '0-0-0-0-0','ffffffff-ffff-ffff-ffff-ffffffffffff','80000000-0000-0000-0000-000000000000','00000000-0000-0000-0000-000000000001']
    for i,uuid in enumerate(uuids):add(f'uuid-{i}',{'text':'Voxrig','hover_event':entity(uuid=uuid)})
    for i,id in enumerate(['pig',':pig','minecraft:pig','player','minecraft:creeper','unknown','','INVALID',1,{}]):add(f'type-{i}',{'text':'Voxrig','hover_event':entity(id=id)})
    for i,name in enumerate(['','Voxrig',{'text':'Voxrig'},{'text':'Voxrig','bold':False},1,{},(9,b'\0\0\0\0\0'),{'text':'Voxrig','click_event':{'action':'run_command','command':'§'}},{'text':'Voxrig','click_event':{'action':'run_command','command':'Voxrig'}},{'text':'Voxrig','click_event':{'action':'open_url','url':'HTTP://HOST:080'}}]):add(f'name-{i}',{'text':'Voxrig','hover_event':entity(name=name)})
    for n in range(128):
        add(f'type-character-{n}',{'text':'Voxrig','hover_event':entity(id='pi'+chr(n)+'g')})
        add(f'uuid-character-{n}',{'text':'Voxrig','hover_event':entity(uuid='1-2-3-4-'+chr(n))})
    for n in [0,1,4,8,12,16,20,32,36,37,64]:
        add(f'uuid-group-length-{n}',{'text':'Voxrig','hover_event':entity(uuid='a'*n+'-2-3-4-5')})
    for i,(id,uuid,name) in enumerate([('pig',u,n) for u in [base_uuid,'00000001-0000-0002-0000-000300000004','1-2-3-4-5'] for n in [None,'Voxrig']]):
        child={'text':'Voxrig','hover_event':entity(id,uuid,name)}
        for kind,fields in [('direct',child),('hover',{'text':'Voxrig','hover_event':{'action':'show_text','value':child}}),('translate',{'translate':'voxrig','with':[child]}),('separator',{'selector':'@a','separator':child}),('siblings',{'text':'Voxrig','extra':[child]}),('nbt',{'nbt':'value','storage':'data','separator':child}),('name',{'text':'Voxrig','hover_event':entity(name=child)})]:add(f'nested-{kind}-{i}',fields)
    for field in ['id','uuid','name']:
        value=entity(name='Voxrig');value.pop(field);add(f'absent-{field}',{'text':'Voxrig','hover_event':value})
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
    sources=['scripts/ExportInventoryTransfers.java','scripts/ExportItemComponents.java','scripts/ExportItemProperties.java','scripts/ExportItemComponentSchema.java','scripts/ExportComponentValueRules.java','scripts/ExportNbtSemantics.java','scripts/ExportComponentNormalization.java','scripts/ExportTextCore.java','scripts/ExportEntityTooltips.java']
    named=b/'named-sources';named.mkdir(exist_ok=True)
    for s in sources:(named/Path(s).name).write_text('package voxrig.oracle;\n'+(ROOT/s).read_text())
    compiler=b/'CompileOwnTool.java';compiler.write_text(COMPILER);classes=b/'own-classes';inputs=b/'inputs.json';raw=b/'raw.json';request_bytes=encoded(requests())
    if not a.normalize_only:
        inputs.write_bytes(request_bytes)
        for label,args in [('compile',[str(compiler),cp,str(classes),*(str(named/Path(s).name) for s in sources)]),('run',['-cp',str(classes)+os.pathsep+cp,'voxrig.oracle.ExportEntityTooltips',str(inputs),str(raw)])]:
            with (b/(label+'.log')).open('w') as log:subprocess.run(['java','-Xmx512M','-XX:ActiveProcessorCount=1',*args],cwd=b,stdout=log,stderr=subprocess.STDOUT,check=True)
    assert inputs.read_bytes()==request_bytes
    data=scalar_json(json.loads(raw.read_text()));rules_file='data/client_api/entity_tooltip_rules-1.21.11.json';rules_bytes=encoded(data['rules']);cases_file='data/client_api/entity_tooltip_cases-1.21.11.json.gz';case_bytes=gzip.compress(encoded({'schema':1,'cases':data['cases'],'pairs':data['pairs']},compact=True),mtime=0)
    source=encoded({'schema':1,'authority':'Unchanged original entity tooltip/text NBT stream codecs, builtin entity type catalog, UUID/name constructor fields, original getters and component.equals; no entity lookup, tooltip rendering, persistent/cache or gameplay proof.','original_server_jar_sha1':sha1,'mappings_sha256':msha,'original_classpath_entries_sha256':classpath,'java_version':data['java_version'],'java_executable_sha256':digest(Path(__import__('shutil').which('java')).resolve().read_bytes()),'jdk_modules_sha256':digest((Path(__import__('shutil').which('java')).resolve().parent.parent/'lib/modules').read_bytes()),'generators_sha256':{s:digest((ROOT/s).read_bytes()) for s in sources+['scripts/export_entity_tooltips.py','scripts/export_text_core.py','scripts/export_item_properties.py','scripts/export_regular_clicks.py']},'generated_compiler_sha256':digest(COMPILER.encode()),'requests_sha256':digest(request_bytes),'raw_output_sha256':digest(raw.read_bytes()),'local_original_entity_tooltip_inspection_sha256':digest((b/'original-entity-tooltip-bytecode.log').read_bytes()),'files_sha256':{rules_file:digest(rules_bytes),cases_file:digest(case_bytes)},'cases':len(data['cases']),'accepted':sum(c['accepted'] for c in data['cases']),'pairs':len(data['pairs'])})
    for name,content in [(rules_file,rules_bytes),(cases_file,case_bytes),('data/client_api/entity_tooltip_source.json',source)]:
        path=ROOT/name
        if a.check:
            if path.read_bytes()!=content:raise SystemExit('native entity tooltip facts differ: '+name)
        else:path.write_bytes(content)
    print('Original entity tooltip facts verified' if a.check else 'Original entity tooltip facts generated',len(data['cases']),sum(c['accepted'] for c in data['cases']),len(data['pairs']))


if __name__=='__main__':main()
