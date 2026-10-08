#!/usr/bin/env python3
"""Record unchanged native URI constructors, fields and equality."""
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
    values = ['', 'http:', 'http:/', 'http://', 'http:///', 'http:///p', 'http:/p', 'http:////p', 'http://?q', 'http://#f', 'http:x', 'HTTP:x', 'https:x', 'ftp:x', 'file:/x', 'mailto:a@b', 'x', '/x', 'http:?q', 'http:#f', 'http:x?y#z', 'http:x#', 'http://host', 'HTTP://HOST', 'http://host/', 'http://host:80', 'http://HOST:080', 'http://host:', 'http://host:-1', 'http://host:abc', 'http://host:2147483647', 'http://host:2147483648', 'http://host:0000000000000000000001', 'http://user:pass@HOST/a?q#f', 'http://user%2f@host/a%2f?q=%ab#%ef', 'http://user%2F@HOST/a%2F?q=%AB#%EF', 'http://host/a/../b', 'http://host/b', 'http://127.0.0.1', 'http://127.000.0.1', 'http://1.2.3', 'http://1.2.3.256', 'http://1', 'http://1.example', 'http://example.1', 'http://a.', 'http://a..b', 'http://-a', 'http://a-', 'http://a_b', 'http://a%41', 'http://日本語', 'http://日本語/a', 'http://u@', 'http://@h', 'http://u@v@h', 'http://[::1]', 'http://[0:0:0:0:0:0:0:1]', 'http://[::FFFF:192.168.0.1]', 'http://[::ffff:192.168.000.1]', 'http://[fe80::1%eth0]', 'http://[FE80::1%ETH0]', 'http://[fe80::1%25eth0]', 'http://[fe80::1%]', 'http://[::]', 'http://[]', 'http://[1:2:3:4:5:6:7:8]', 'http://[1:2:3:4:5:6:7]', 'http://[1:2:3:4:5:6:7:8:9]', 'http://[1::2::3]', 'http://[v1.foo]', 'http://[::1]:abc', 'http://[::1]:', 'http://[::1]:080', 'http://[::1]extra', 'http://h?x', 'http://h?X', 'http://h#x', 'http://h#X', 'http://h/%41', 'http://h/A', 'http://h/%', 'http://h/%0', 'http://h/%GG', 'http://h/a#b#c']
    shapes=['http:x{}y','http://h/x{}y','http://h/?x{}y','http://h/#x{}y','http://u{}v@h/','http://h{}i/','http://[fe80::1%a{}b]/','h{}ttp://h/']
    for code in list(range(128))+[128,159,160,5760,8192,8203,8232,8233,8239,8287,12288,0xd800,0xdc00,0xffff,0x1f48e]:
        for shape in shapes: values.append(shape.format(chr(code)))
    for host in ['0000000000000001.2.3.4', '255.255.255.255', '256.0.0.1', '1.2.3.04', '1.2.3.', '.1.2.3', '1.2.3.4.5', 'A.B.1', '123.', 'a.b.', 'a.-b', 'a_b.c', '%31.2.3.4', 'localhost', '[::192.168.0.1]', '[::192.168.0000000000001.1]', '[1:2:3:4:5:6:192.168.0.1]', '[1:2:3:4:5:192.168.0.1]', '[1:2:3:4:5::192.168.0.1]', '[fe80::1%a_b]', '[fe80::1%a.b]', '[fe80::1%a-b]', '[fe80::1%日本語]', '[fe80::1%a%25b]']:
        values.append('http://'+host+'/')
    for token in ['', '0', '000', '+1', '-0', '2147483647', '2147483648', '00000000000000000000000000001', '１２', '%31', '1:2', '1@h', '1;2', '1.0']:
        for host in ['h', '[::1]']: values.append('http://'+host+':'+token+'/')
    for count in range(0,10):
        address=':'.join(['1']*count)
        for probe in [address, address+'::', '::'+address]: values.append('http://['+probe+']/')
    for host in ['-a.b', 'a-.b', 'a.b-', 'a.b_c', 'a.1b', 'a.1', 'a.0x1', 'a..', 'a.b..', 'a..b.', '.', '..', '0.', '1.2.3.4.', 'a:b:c', ':1', '[::1%_]', '[::1%.]', '[::1%-]', '[::1%a:b]', '[::1%a%25b]', '[::1%abc123]', '[::1%Ａ]', '[::ffff:0001.2.3.4]', '[00001::]', '[::00001]', '[::+1]', '[::-1]', '[::1.2.3.256]', '[::1.2.3]', '[::1.2.3.4.5]']: values.append('http://'+host+'/')
    for escape in ['%2f','%2F','%ＦＦ','%aG','%A0','%00','%d8%00','%ff','%FF','%25','%%20']:
        for shape in ['http://u{}@h/', 'http://h/a{}', 'http://h?{}', 'http:x#{}']: values.append(shape.format(escape))
    rows=[{'case':f'uri-{i}', 'kind':'uri', 'value':v} for i,v in enumerate(values)]
    for i,value in enumerate(values[:83]):
        child={'text':'Voxrig','click_event':{'action':'open_url','url':value}}
        for name,fields in [('direct',child),('hover',{'text':'Voxrig','hover_event':{'action':'show_text','value':child}}),('translate',{'translate':'voxrig','with':[child]}),('separator',{'selector':'@a','separator':child}),('siblings',{'text':'Voxrig','extra':[child]})]:
            kind,payload=tag(fields);rows.append({'case':f'text-{name}-{i}','kind':'text','input_hex':(bytes([kind])+payload).hex()})
    for i,value in enumerate(['http:x','http://host','http://','ftp:x','http://h/%','HTTP://HOST:080']):
        child={'text':'Voxrig','click_event':{'action':'open_url','url':value}}
        fields={'nbt':'value','storage':'data','separator':child};kind,payload=tag(fields);rows.append({'case':f'text-nbt-{i}','kind':'text','input_hex':(bytes([kind])+payload).hex()})
    for value in [1,{},False]:
        fields={'text':'Voxrig','click_event':{'action':'open_url','url':value}};kind,payload=tag(fields);rows.append({'case':f'text-invalid-{len(rows)}','kind':'text','input_hex':(bytes([kind])+payload).hex()})
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
    sources=['scripts/ExportInventoryTransfers.java','scripts/ExportItemComponents.java','scripts/ExportItemProperties.java','scripts/ExportItemComponentSchema.java','scripts/ExportComponentValueRules.java','scripts/ExportNbtSemantics.java','scripts/ExportComponentNormalization.java','scripts/ExportTextCore.java','scripts/ExportUris.java']
    named=b/'named-sources';named.mkdir(exist_ok=True)
    for s in sources:(named/Path(s).name).write_text('package voxrig.oracle;\n'+(ROOT/s).read_text())
    compiler=b/'CompileOwnTool.java';compiler.write_text(COMPILER);classes=b/'own-classes';inputs=b/'inputs.json';raw=b/'raw.json';request_bytes=encoded(requests())
    if not a.normalize_only:
        inputs.write_bytes(request_bytes)
        for label,args in [('compile',[str(compiler),cp,str(classes),*(str(named/Path(s).name) for s in sources)]),('run',['-cp',str(classes)+os.pathsep+cp,'voxrig.oracle.ExportUris',str(inputs),str(raw)])]:
            with (b/(label+'.log')).open('w') as log:subprocess.run(['java','--add-opens=java.base/java.net=ALL-UNNAMED','-Xmx512M','-XX:ActiveProcessorCount=1',*args],cwd=b,stdout=log,stderr=subprocess.STDOUT,check=True)
    assert inputs.read_bytes()==request_bytes
    data=scalar_json(json.loads(raw.read_text()));rules_file='data/client_api/uri_rules-1.21.11.json';rules_bytes=encoded(data['rules']);cases_file='data/client_api/uri_cases-1.21.11.json.gz';case_bytes=gzip.compress(encoded({'schema':1,'cases':data['cases'],'pairs':data['pairs']},compact=True),mtime=0)
    source=encoded({'schema':1,'authority':'Unchanged original Minecraft URI/text codecs, Java URI getters/equality and original ASCII masks/non-ASCII exclusions/allowed schemes; no URL opening, network access, persistent/cache or gameplay proof.','original_server_jar_sha1':sha1,'mappings_sha256':msha,'original_classpath_entries_sha256':classpath,'java_version':data['java_version'],'java_executable_sha256':digest(Path(__import__('shutil').which('java')).resolve().read_bytes()),'jdk_modules_sha256':digest((Path(__import__('shutil').which('java')).resolve().parent.parent/'lib/modules').read_bytes()),'generators_sha256':{s:digest((ROOT/s).read_bytes()) for s in sources+['scripts/export_uris.py','scripts/export_text_core.py','scripts/export_item_properties.py','scripts/export_regular_clicks.py']},'generated_compiler_sha256':digest(COMPILER.encode()),'requests_sha256':digest(request_bytes),'raw_output_sha256':digest(raw.read_bytes()),'local_original_uri_inspection_sha256':digest((b/'original-uri-bytecode.log').read_bytes()),'files_sha256':{rules_file:digest(rules_bytes),cases_file:digest(case_bytes)},'cases':len(data['cases']),'accepted':sum(c['accepted'] for c in data['cases']),'pairs':len(data['pairs'])})
    for name,content in [(rules_file,rules_bytes),(cases_file,case_bytes),('data/client_api/uri_source.json',source)]:
        path=ROOT/name
        if a.check:
            if path.read_bytes()!=content:raise SystemExit('native URI facts differ: '+name)
        else:path.write_bytes(content)
    print('Original URI facts verified' if a.check else 'Original URI facts generated',len(data['cases']),sum(c['accepted'] for c in data['cases']),len(data['pairs']))


if __name__=='__main__':main()
