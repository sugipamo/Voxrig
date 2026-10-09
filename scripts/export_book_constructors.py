#!/usr/bin/env python3
"""Record unchanged native book/enchantability constructors, fields and equality."""
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
    from export_text_core import tag
    def varint(value):
        value &= 0xffffffff
        out=bytearray()
        while value > 127:
            out.append((value & 127) | 128); value >>= 7
        out.append(value)
        return bytes(out)
    def string(value):
        wire=value.encode('utf-8')
        return varint(len(wire))+wire
    def text(value):
        kind,wire=tag(value)
        return bytes([kind])+wire
    def filtered(raw,filtered=None,encoder=string):
        return encoder(raw)+(b'\0' if filtered is None else b'\1'+encoder(filtered))
    def writable(pages):
        return varint(len(pages))+b''.join(filtered(*p) for p in pages)
    def written(title=('',None),author='',generation=0,pages=(),resolved=0):
        return filtered(*title)+string(author)+varint(generation)+varint(len(pages))+b''.join(filtered(*p,encoder=text) for p in pages)+bytes([resolved])
    rows=[]
    def add(component,name,wire):
        rows.append({'case':component+'-'+name,'component':'minecraft:'+component,'input_hex':wire.hex()})
    for value in [-2147483648,-255,-1,0,1,2,3,4,127,128,255,256,2147483647]:
        add('enchantable','value-'+str(value),varint(value))
    for wire in [b'\x81\0',b'\x81\x80\0',b'',b'\x80',b'\1\0']:
        add('enchantable','wire-'+wire.hex(),wire)
    add('writable_book_content','empty',writable([]))
    for raw,other in [('',None),('', ''),('page',None),('page','page'),('page','filtered'),('日本語💎',None),('a\0b',None),('x'*1024,None),('x'*1025,None),('x','x'*1024),('x','x'*1025)]:
        add('writable_book_content','page-'+str(len(rows)),writable([(raw,other)]))
    for count in [1,2,99,100,101]:
        add('writable_book_content','count-'+str(count),writable([('a',None)]*count))
    for pages in [[('a',None),('b',None)],[('b',None),('a',None)],[('a','b'),('c','d')]]:
        add('writable_book_content','order-'+str(len(rows)),writable(pages))
    for value in [-2147483648,-1,0,1,2,3,4,127,255,2147483647]:
        add('written_book_content','generation-'+str(value),written(generation=value))
    for title in [('',None),('', ''),('Title',None),('Title','Title'),('Title','filtered'),('x'*32,None),('x'*33,None),('x','x'*32),('x','x'*33),('💎'*16,None),('💎'*17,None)]:
        add('written_book_content','title-'+str(len(rows)),written(title=title))
    for author in ['', 'Author','日本語💎','a\0b','x'*32767,'x'*32768]:
        add('written_book_content','author-'+str(len(rows)),written(author=author))
    for resolved in [0,1,2,127,255]:
        add('written_book_content','resolved-'+str(resolved),written(resolved=resolved))
    for raw,other in [('a',None),({'text':'a'},None),({'text':'a','bold':0},None),({'text':'a','bold':1},None),('a','a'),('a',{'text':'a'}),('a','b'),({'translate':'voxrig.message','with':['a']},None),('日本語💎',None),('a\0b',None),('\ud800',None)]:
        add('written_book_content','page-'+str(len(rows)),written(pages=[(raw,other)]))
    for count in [0,1,2,100,101]:
        add('written_book_content','count-'+str(count),written(pages=[('a',None)]*count))
    for pages in [[('a',None),('b',None)],[('b',None),('a',None)],[('a','b'),('c','d')]]:
        add('written_book_content','order-'+str(len(rows)),written(pages=pages))
    # Observe every truncation of a complete sample and trailing-envelope bytes.
    for component,wire in [('writable_book_content',writable([('a','b')])),('written_book_content',written(title=('a','b'),author='c',generation=1,pages=[('d','e')],resolved=1))]:
        for length in range(len(wire)):
            add(component,'truncated-'+str(length),wire[:length])
        add(component,'trailing',wire+b'\0')
    add('writable_book_content','noncanonical-empty',b'\x80\0')
    add('written_book_content','noncanonical-generation',b'\0\0\0\x80\0\0\0')
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
    sources=['scripts/ExportInventoryTransfers.java','scripts/ExportItemComponents.java','scripts/ExportItemProperties.java','scripts/ExportItemComponentSchema.java','scripts/ExportComponentValueRules.java','scripts/ExportNbtSemantics.java','scripts/ExportComponentNormalization.java','scripts/ExportTextCore.java','scripts/ExportEnchantmentConstructors.java','scripts/ExportBookConstructors.java']
    named=b/'named-sources';named.mkdir(exist_ok=True)
    for s in sources:(named/Path(s).name).write_text('package voxrig.oracle;\n'+(ROOT/s).read_text())
    compiler=b/'CompileOwnTool.java';compiler.write_text(COMPILER);classes=b/'own-classes';inputs=b/'inputs.json';raw=b/'raw.json';request_bytes=encoded(requests())
    if not a.normalize_only:
        inputs.write_bytes(request_bytes)
        for label,args in [('compile',[str(compiler),cp,str(classes),*(str(named/Path(s).name) for s in sources)]),('run',['-cp',str(classes)+os.pathsep+cp,'voxrig.oracle.ExportBookConstructors',str(inputs),str(raw)])]:
            with (b/(label+'.log')).open('w') as log:subprocess.run(['java','-Xmx512M','-XX:ActiveProcessorCount=1',*args],cwd=b,stdout=log,stderr=subprocess.STDOUT,check=True)
    assert inputs.read_bytes()==request_bytes
    data=scalar_json(json.loads(raw.read_text()));rules_file='data/client_api/book_constructor_rules-1.21.11.json';rules_bytes=encoded({'enchantable':{'forward':468,'child':2,'minimum':1},'written_book':{'sequence':529,'children':[530,14,2,533,5],'generation_minimum':0,'generation_maximum':3},'writable_book':{'forward':524,'child':525,'maximum_pages':100},'filterable_string_nodes':[526,530],'filterable_text_node':534,'written_pages_stream_maximum':2147483647});cases_file='data/client_api/book_constructor_cases-1.21.11.json.gz';case_bytes=gzip.compress(encoded({'schema':1,'cases':data['cases'],'pairs':data['pairs']},compact=True),mtime=0)
    source=encoded({'schema':1,'authority':'Unchanged original writable/written book and enchantability stream codecs, constructor acceptance, raw/filtered fields, text getters, original encoded streams and value.equals under observed vanilla registry context. Text reference dependencies, legacy books, persistent encoding, server cache and action admission remain separate obligations.','original_server_jar_sha1':sha1,'mappings_sha256':msha,'original_classpath_entries_sha256':classpath,'java_version':data['java_version'],'java_executable_sha256':digest(Path(__import__('shutil').which('java')).resolve().read_bytes()),'jdk_modules_sha256':digest((Path(__import__('shutil').which('java')).resolve().parent.parent/'lib/modules').read_bytes()),'generators_sha256':{s:digest((ROOT/s).read_bytes()) for s in sources+['scripts/export_book_constructors.py','scripts/export_text_core.py','scripts/export_item_properties.py','scripts/export_regular_clicks.py']},'generated_compiler_sha256':digest(COMPILER.encode()),'requests_sha256':digest(request_bytes),'raw_output_sha256':digest(raw.read_bytes()),'local_original_book_constructor_inspection_sha256':digest((b/'original-books-bytecode.log').read_bytes()),'files_sha256':{rules_file:digest(rules_bytes),cases_file:digest(case_bytes)},'cases':len(data['cases']),'accepted':sum(c['accepted'] for c in data['cases']),'pairs':len(data['pairs'])})
    for name,content in [(rules_file,rules_bytes),(cases_file,case_bytes),('data/client_api/book_constructor_source.json',source)]:
        path=ROOT/name
        if a.check:
            if path.read_bytes()!=content:raise SystemExit('native book constructor facts differ: '+name)
        else:path.write_bytes(content)
    print('Original book constructor facts verified' if a.check else 'Original book constructor facts generated',len(data['cases']),sum(c['accepted'] for c in data['cases']),len(data['pairs']))


if __name__=='__main__':main()
