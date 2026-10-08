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
    rows=[]
    def add(name,fields):
        kind,payload=tag(fields);rows.append({'case':name,'input_hex':(bytes([kind])+payload).hex()})
    for action,field in [('run_command','command'),('suggest_command','command'),('copy_to_clipboard','value')]:
        for code in list(range(174))+[5760,8192,8203,8232,8233,12288,0xd800,0xdc00,0xffff,0x1f48e]:
            add(f'{action}-unit-{code}',{'text':'Voxrig','click_event':{'action':action,field:'A'+chr(code)+'B'}})
        for index,value in enumerate(['','/say Voxrig',' 日本語 ','\ud800','§','line\nbreak',1,{},False]):
            add(f'{action}-scalar-{index}',{'text':'Voxrig','click_event':{'action':action,field:value}})
    for index,value in enumerate([1,0,-1,2147483647,2147483648,(4,__import__('struct').pack('>q',4294967297)),1.9,0.9,float('nan'),float('inf'),float('-inf'),'1',{},True]):
        add(f'page-{index}',{'text':'Voxrig','click_event':{'action':'change_page','page':value}})
    for index,value in enumerate([None,{},'Voxrig',1,1.0,False,(9,b'\0\0\0\0\0'),{'a':1,'b':2},{'b':2,'a':1},{'a':1.0},(5,__import__('struct').pack('>I',0x7fc00001)),(5,__import__('struct').pack('>I',0x7fc00002))]):
        fields={'action':'custom','id':'voxrig:event'}
        if value is not None:fields['payload']=value
        add(f'custom-{index}',{'text':'Voxrig','click_event':fields})
    for i,value in enumerate(['event',':event','minecraft:event','voxrig:event','','INVALID',1,{}]):
        add(f'custom-id-{i}',{'text':'Voxrig','click_event':{'action':'custom','id':value}})
    for i,value in enumerate(['default',':default','minecraft:default','','INVALID',1,{}, {'atlas':'blocks','sprite':'stone'},{'player':'Voxrig'}]):
        add(f'font-{i}',{'text':'Voxrig','font':value})
    for action in ['run_command','suggest_command','copy_to_clipboard']:
        field='value' if action=='copy_to_clipboard' else 'command'
        for i,value in enumerate(['Voxrig','§','line\nbreak','']):
            child={'text':'Voxrig','click_event':{'action':action,field:value}}
            for kind,fields in [('hover',{'text':'Voxrig','hover_event':{'action':'show_text','value':child}}),('translate',{'translate':'voxrig','with':[child]}),('separator',{'selector':'@a','separator':child}),('siblings',{'text':'Voxrig','extra':[child]}),('nbt',{'nbt':'value','storage':'data','separator':child})]:add(f'nested-{action}-{kind}-{i}',fields)
    for source,values in [('entity',['@a','@e[limit=0]','@bad','a'*17,'']),('block',['~ ~ ~','invalid','1 2','', '1 2 3 trailing']),('storage',['data',':data','minecraft:data','','INVALID'])]:
        for i,value in enumerate(values):add(f'nbt-source-{source}-{i}',{'nbt':'[invalid','interpret':True,source:value})
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
    sources=['scripts/ExportInventoryTransfers.java','scripts/ExportItemComponents.java','scripts/ExportItemProperties.java','scripts/ExportItemComponentSchema.java','scripts/ExportComponentValueRules.java','scripts/ExportNbtSemantics.java','scripts/ExportComponentNormalization.java','scripts/ExportTextCore.java','scripts/ExportClickConstructors.java']
    named=b/'named-sources';named.mkdir(exist_ok=True)
    for s in sources:(named/Path(s).name).write_text('package voxrig.oracle;\n'+(ROOT/s).read_text())
    compiler=b/'CompileOwnTool.java';compiler.write_text(COMPILER);classes=b/'own-classes';inputs=b/'inputs.json';raw=b/'raw.json';request_bytes=encoded(requests())
    if not a.normalize_only:
        inputs.write_bytes(request_bytes)
        for label,args in [('compile',[str(compiler),cp,str(classes),*(str(named/Path(s).name) for s in sources)]),('run',['-cp',str(classes)+os.pathsep+cp,'voxrig.oracle.ExportClickConstructors',str(inputs),str(raw)])]:
            with (b/(label+'.log')).open('w') as log:subprocess.run(['java','-Xmx512M','-XX:ActiveProcessorCount=1',*args],cwd=b,stdout=log,stderr=subprocess.STDOUT,check=True)
    assert inputs.read_bytes()==request_bytes
    data=scalar_json(json.loads(raw.read_text()));rules_file='data/client_api/click_constructor_rules-1.21.11.json';rules_bytes=encoded(data['rules']);cases_file='data/client_api/click_constructor_cases-1.21.11.json.gz';case_bytes=gzip.compress(encoded({'schema':1,'cases':data['cases'],'pairs':data['pairs']},compact=True),mtime=0)
    source=encoded({'schema':1,'authority':'Unchanged original click/text NBT stream codecs, original chat UTF-16 exclusion predicate, click getters and component.equals, font resource/source constructor probes; no command execution, persistent/cache or gameplay proof.','original_server_jar_sha1':sha1,'mappings_sha256':msha,'original_classpath_entries_sha256':classpath,'java_version':data['java_version'],'java_executable_sha256':digest(Path(__import__('shutil').which('java')).resolve().read_bytes()),'jdk_modules_sha256':digest((Path(__import__('shutil').which('java')).resolve().parent.parent/'lib/modules').read_bytes()),'generators_sha256':{s:digest((ROOT/s).read_bytes()) for s in sources+['scripts/export_click_constructors.py','scripts/export_text_core.py','scripts/export_item_properties.py','scripts/export_regular_clicks.py']},'generated_compiler_sha256':digest(COMPILER.encode()),'requests_sha256':digest(request_bytes),'raw_output_sha256':digest(raw.read_bytes()),'local_original_click_inspection_sha256':digest((b/'original-click-bytecode.log').read_bytes()),'files_sha256':{rules_file:digest(rules_bytes),cases_file:digest(case_bytes)},'cases':len(data['cases']),'accepted':sum(c['accepted'] for c in data['cases']),'pairs':len(data['pairs'])})
    for name,content in [(rules_file,rules_bytes),(cases_file,case_bytes),('data/client_api/click_constructor_source.json',source)]:
        path=ROOT/name
        if a.check:
            if path.read_bytes()!=content:raise SystemExit('native click constructor facts differ: '+name)
        else:path.write_bytes(content)
    print('Original click constructor facts verified' if a.check else 'Original click constructor facts generated',len(data['cases']),sum(c['accepted'] for c in data['cases']),len(data['pairs']))


if __name__=='__main__':main()
