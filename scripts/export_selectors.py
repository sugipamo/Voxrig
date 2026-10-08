#!/usr/bin/env python3
"""Record unchanged native selector/score construction, options and equality."""
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
    patterns = ['', 'Voxrig', '*', '@', '@p', '@a', '@r', '@s', '@e', '@n', '@x', '@P', '@p[]', '@p[ ]', '@p[', '@p]', '@p[]tail', '@p trailing', '@p [limit=0]', 'Voxrig trailing', ' Voxrig', '\t@a', '1234567890123456', '12345678901234567', '1-2-3-4-5', '00000001-0000-0002-0000-000300000004', '00000001-0000-0002-0000-000300000004suffix', '日本語', '"日本語"', '"💎"', '"\ud800"', "'Voxrig'", '""', '"a\\b"', '"a\\\"b"']
    for c in range(128):
        patterns += ['A' + chr(c) + 'B', '@e[name=A' + chr(c) + 'B]']
    probes = {
        'name':['', 'Voxrig', '!Voxrig', '!!Voxrig', '"a b"', '!"a b"', '日本語', '"日本語"', '"\\q"'],
        'distance':['', '0', '-1', '1..2', '2..1', '..2', '1..', '..', '-1..2', '1.5', '.5', '1e2', '+1', 'NaN', 'Infinity', '1..2..3'],
        'level':['', '0', '-1', '1..2', '2..1', '..2', '1..', '..', '1.5', '2147483647', '2147483648'],
        'limit':['', '0', '-1', '1', '2147483647', '2147483648', '1.0', '+1'],
        'sort':['nearest','furthest','random','arbitrary','NEAREST','"nearest"',''],
        'gamemode':['survival','creative','adventure','spectator','!survival','survival_mode','SURVIVAL','"survival"',''],
        'team':['','!','red','!red','"a b"','日本語'],
        'type':['pig','minecraft:pig','player','!pig','!player','#unknown','!#unknown','unknown','INVALID',''],
        'tag':['','!','test','!test','"a b"'],
        'nbt':['{}','!{}','{a:1}','{a:[1,2]}','{a:[B;1b,2b]}','{a:[I;1,2]}','{a:[L;1L,2L]}','{a:[1,"x"]}','{a:true}','{a:null}','{a:NaN}','{a:Infinity}','{a:1e2}','{a:"x"}','{a:"\\q"}','{a:}','','[]','![]','{a:{b:{c:1}}}'],
        'scores':['{}','{foo=1}','{foo=1..2}','{foo=2..1}','{foo=-1}','{foo=..}','{foo=1,bar=2}','{foo=1,}','{=1}','{"a b"=1}','{foo=1.0}','{foo=1,foo=2}',''],
        'advancements':['{}','{story/root=true}','{story/root=false}','{story/root={}}','{story/root={test=true}}','{story/root={test=false,other=true}}','{story/root=1}','{INVALID=true}','{story/root=true,}',''],
        'predicate':['foo','!foo','minecraft:foo','#foo','!#foo','INVALID',''],
        'unknown':['1'],
    }
    for key in ['x','y','z','dx','dy','dz']:
        probes[key]=['','0','-1','1.5','.5','1e2','+1','NaN','Infinity','9999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999999']
    for key in ['x_rotation','y_rotation']:
        probes[key]=['','0','-180','180','360','-360','-190..190','1..2','2..1','..','..1','1..','1.5','1e2','NaN']
    for key,values in probes.items():
        for value in values:
            patterns += [f'@e[{key}={value}]', f'@e[ {key} = {value} , ]']
    pairs=['name=foo,name=bar','name=!foo,name=!bar','name=foo,name=!bar','name=!foo,name=bar','gamemode=survival,gamemode=creative','gamemode=!survival,gamemode=!creative','team=red,team=!blue','team=!red,team=blue','type=pig,type=cow','type=!pig,type=cow','type=#foo,type=#bar','type=player,type=!pig','limit=1,limit=2','sort=nearest,sort=random','x=1,x=2','distance=1,distance=2','level=1,level=2','scores={},scores={}','scores={a=1},scores={b=2}','advancements={},advancements={}','tag=a,tag=b','nbt={},nbt={}','predicate=foo,predicate=bar']
    for selector in ['@e','@a','@s','@p','@n']:
        patterns += [f'{selector}[{pair}]' for pair in pairs]
    patterns += ['@e[limit=1,,]','@e[limit=1 name=x]','@e["name"="x"]','@e[limit=1,]','@e[limit=1]ignored','@e[type=minecraft:creeper]','@e[type=!#minecraft:undead,name=!Voxrig,distance=..32,limit=4,sort=nearest]']
    # Probe every SNBT scalar/list/typed-array constructor used by selectors.
    atoms=['255ub','65535us','4294967295ui','18446744073709551615ul','255b','1i','+1','0b101','0xFF','0o10','1__0','_1','1_','1e2','1.0e+2','1','1b','128b','-129b','32768s','1s','1L','2147483648','9223372036854775808L','1.5','1.5f','NaNf','Infinityf','NaN','null','true','false','\"text\"','\"日本語\"','{}','[]','[1,2]','[1,\"x\"]','0x1','1_000','-0.0d','1e2d']
    for prefix in ['','B;','I;','L;']:
        for atom in atoms:
            patterns += ['@e[nbt={a:['+prefix+atom+']}]','@e[nbt={a:['+prefix+atom+','+atom+',]}]']
    for value in ['{a:1,}','{\"a\":1}','{a b:1}','{a:1,a:2}','{a:[1,{b:2}]}','{a:[B;]}','{a:[B;1,2]}','{a:[I;1b,2L]}','{a:[L;1,2]}','{a:1}ignored','{a:(1)}','{a:true()}', '{a:bool(1)}']:
        patterns.append('@e[nbt='+value+']')
    # Noncanonical ByteBuf boolean bytes at each distinct profile path.
    def wire_string(value):
        value=value.encode('utf-8');return bytes([len(value)])+value
    uid=struct.pack('>IIII',1,2,3,4)
    profile_rows=[]
    for bit in [0,1,2,127,128,255]:
        shapes={
            'either':bytes([bit])+uid+wire_string('Voxrig')+b'\0\0\0\0\0',
            'name_optional':b'\0'+bytes([bit])+wire_string('Voxrig')+b'\0\0\0\0\0\0',
            'uuid_optional':b'\0\0'+bytes([bit])+uid+b'\0\0\0\0\0',
            'texture_optional':b'\0\0\0\0'+bytes([bit])+wire_string('stone')+b'\0\0\0',
            'model_optional':b'\0\0\0\0\0\0\0'+bytes([bit])+b'\1',
            'signature_optional':b'\0\0\0\1'+wire_string('a')+wire_string('b')+bytes([bit])+wire_string('s')+b'\0\0\0\0',
        }
        for name,wire in shapes.items():
            # false is checked with its actual absent payload; do not mistake
            # own trailing-field assertions for original constructor failures.
            if bit==0:continue
            profile_rows.append({'case':f'profile-{name}-{bit}','kind':'profile','input_hex':wire.hex()})
    for value in ['bool(true)','bool(false)','bool(1)','bool(0)','bool(1.5)','bool("true")','bool("x")','bool([])','bool(1,2)','uuid("00000001-0000-0002-0000-000300000004")','uuid("1-2-3-4-5")','uuid("invalid")','uuid(1)', 'unknown(1)', '"a\\n"','"a\\t"','"a\\u0041"','"a\\x41"','"a\\q"']:
        patterns.append('@e[nbt={a:'+value+'}]')
    for pair in ['type=pig','type=!pig','type=#foo','type=!#foo','limit=1','sort=arbitrary','gamemode=survival','level=1']:
        for origin in ['@a','@p','@r','@s','@n']:
            patterns.append(origin+'['+pair+']')
    # Independent native edge corpus for the full SNBT string/number grammar.
    for escape in ['0','a','b','e','f','n','r','s','t','v','u0041','U0001F48E','U00110000','U0000D800','x41','x0','u12G4','N{LATIN CAPITAL LETTER A}','N{latin capital letter a}','N{latin capıtal letter a}','N{ROMAN NUMERAL ﬁFTY}','N{LEß-THAN SIGN}','N{HIGH SURROGATES D800}','N{CJK UNIFIED IDEOGRAPHS 4E00}','N{CJK UNIFIED IDEOGRAPH-4E00}','N{PRIVATE USE AREA E000}','N{PRIVATE USE AREA 0000}','N{invalid}','N{ NULL }']:
        patterns.append('@e[nbt={a:"x'+chr(92)+escape+'"}]')
    for c in [0,8,9,10,13,27,31,127,128]:
        patterns.append('@e[nbt={a:"A'+chr(c)+'B"}]')
    for token in ['01','00','0.1','01.0','-01','+01','0xFF','0xffb','0xffsb','0xffub','0xffs','0x80000000','0xFFFFFFFF','-0x80','+0xFF','-0x81b','0B10','0XFF','01f','1f','1D','1F','1u','1sb','-1ub','1.0b','1e999','1e999f','1e999d','1e50f','1.e2','.5e2','1e+2','1e-2','1_e2','1e_2','1e2_','1_.0','1._0','0x_1','0x1_','0b_1','1 b','1 . 5','bool (1)','uuid ("1-2-3-4-5")']:
        for target in ['', 'B;', 'I;', 'L;']:
            patterns.append('@e[nbt={a:['+target+token+']}]')
    for token in ['bool(NaN)','bool(NaNf)','bool(Infinityf)','bool(0xFF)','bool(true,)', 'uuid("１-２-３-４-５")','uuid("０００００００１-００００-０００２-００００-０００３０００００００４")','uuid("+1-+2-+3-+4-+5")']:
        patterns.append('@e[nbt={a:'+token+'}]')
    for left in ['pig','!pig','#foo','!#foo','player']:
        for right in ['cow','!cow','#bar','!#bar']:
            patterns.append('@e[type='+left+',type='+right+']')
    for token in ['1 2','1 e 2','1e 2','1 e + 2','1e +2','- 1','+ 1','. 5','1. 5','1 .5','1 u b','1 ub','1 uB','1 Ub','1 s b','0 x FF','0x FF','0xFF b','0b 10','0 b 10','1_ 2','1 _2','1 b b','1 f d']:
        for prefix in ['', 'B;', 'I;', 'L;']:
            patterns.append('@e[nbt={a:['+prefix+token+']}]')
    for quoted in ['"a"', '"a'+chr(92)+chr(39)+'b"', chr(39)+'a'+chr(92)+chr(34)+'b'+chr(39), '"'+chr(92)+'N{LATIN_CAPITAL_LETTER_A}"', '"'+chr(92)+'N{LATIN CAPITAL LETTER A'+chr(9)+'}"']:
        patterns.append('@e[nbt={a:'+quoted+'}]')
    for key in ['1', '+1', '.1', '-1', '1_0', 'true', '""', '日本語', '"日本語"']:
        patterns.append('@e[nbt={'+key+':1}]')
    rows=[{'case':f'selector-{i}' , 'kind':'selector','pattern':pattern} for i,pattern in enumerate(patterns)]
    for i,pattern in enumerate(patterns[:35] + [f'@e[{key}={values[-1]}]' for key,values in probes.items()] + ['@e[limit=0]','@e[type=unknown]','@e[scores={a=2..1}]']):
        for kind,fields in [('text', {'selector':pattern}), ('score', {'score':{'name':pattern,'objective':'voxrig'}}), ('fuzzy',{'selector':pattern,'keybind':'key.jump'}), ('explicit',{'type':'selector','selector':pattern,'keybind':'key.jump'}), ('fuzzy-nbt',{'selector':pattern,'nbt':'value','storage':'data'})]:
            tag_type,payload=tag(fields)
            rows.append({'case':f'{kind}-{i}','kind':'text','input_hex':(bytes([tag_type])+payload).hex()})
    return rows+profile_rows


def main():
    p=argparse.ArgumentParser(description=__doc__)
    for name in ['downloads','modern-classpath-file','runtime-output']:
        p.add_argument('--'+name,type=Path,required=True)
    p.add_argument('--normalize-only',action='store_true');p.add_argument('--check',action='store_true')
    a=p.parse_args();b=a.runtime_output.resolve();b.mkdir(parents=True,exist_ok=True)
    jar=(a.downloads/'1.21.11-server.jar').resolve();mapping=a.downloads/'1.21.11-server-mappings.txt';sha1,msha=VERSIONS['1.21.11']
    if digest(jar.read_bytes(),'sha1')!=sha1 or digest(mapping.read_bytes())!=msha:raise SystemExit('original inputs differ')
    cp=os.pathsep.join(str(Path(s).resolve()) for s in a.modern_classpath_file.read_text().strip().split(os.pathsep));classpath=original_modern_classpath(jar,cp)
    sources=['scripts/ExportInventoryTransfers.java','scripts/ExportItemComponents.java','scripts/ExportItemProperties.java','scripts/ExportItemComponentSchema.java','scripts/ExportComponentValueRules.java','scripts/ExportNbtSemantics.java','scripts/ExportComponentNormalization.java','scripts/ExportTextCore.java','scripts/ExportProfiles.java','scripts/ExportSelectors.java']
    named=b/'named-sources';named.mkdir(exist_ok=True)
    for s in sources:(named/Path(s).name).write_text('package voxrig.oracle;\n'+(ROOT/s).read_text())
    compiler=b/'CompileOwnTool.java';compiler.write_text(COMPILER);classes=b/'own-classes';inputs=b/'inputs.json';raw=b/'raw.json';request_bytes=encoded(requests())
    if not a.normalize_only:
        inputs.write_bytes(request_bytes)
        for label,args in [('compile',[str(compiler),cp,str(classes),*(str(named/Path(s).name) for s in sources)]),('run',['-cp',str(classes)+os.pathsep+cp,'voxrig.oracle.ExportSelectors',str(inputs),str(raw)])]:
            with (b/(label+'.log')).open('w') as log:subprocess.run(['java','-Xmx512M','-XX:ActiveProcessorCount=1',*args],cwd=b,stdout=log,stderr=subprocess.STDOUT,check=True)
    assert inputs.read_bytes()==request_bytes
    data=scalar_json(json.loads(raw.read_text()));rules_file='data/client_api/selector_rules-1.21.11.json';rules_bytes=encoded(data['rules']);cases_file='data/client_api/selector_cases-1.21.11.json.gz';case_bytes=gzip.compress(encoded({'schema':1,'cases':data['cases'],'pairs':data['pairs']},compact=True),mtime=0)
    source=encoded({'schema':1,'authority':'Unchanged original SelectorPattern/text constructor codecs, parsed getter/cursor fields, native equals and original option/entity type catalogs; no entity resolution, persistent/cache or gameplay proof.','original_server_jar_sha1':sha1,'mappings_sha256':msha,'original_classpath_entries_sha256':classpath,'java_version':data['java_version'],'generators_sha256':{s:digest((ROOT/s).read_bytes()) for s in sources+['scripts/export_selectors.py','scripts/export_text_core.py','scripts/export_item_properties.py','scripts/export_regular_clicks.py']},'generated_compiler_sha256':digest(COMPILER.encode()),'requests_sha256':digest(request_bytes),'raw_output_sha256':digest(raw.read_bytes()),'local_original_selector_inspection_sha256':digest((b/'original-selector-bytecode.log').read_bytes()),'files_sha256':{rules_file:digest(rules_bytes),cases_file:digest(case_bytes)},'cases':len(data['cases']),'accepted':sum(c['accepted'] for c in data['cases']),'pairs':len(data['pairs'])})
    for name,content in [(rules_file,rules_bytes),(cases_file,case_bytes),('data/client_api/selector_source.json',source)]:
        path=ROOT/name
        if a.check:
            if path.read_bytes()!=content:raise SystemExit('native selector facts differ: '+name)
        else:path.write_bytes(content)
    print('Original selector facts verified' if a.check else 'Original selector facts generated',len(data['cases']),sum(c['accepted'] for c in data['cases']),len(data['pairs']))


if __name__=='__main__':main()
