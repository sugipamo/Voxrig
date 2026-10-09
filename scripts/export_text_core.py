#!/usr/bin/env python3
"""Observe original text getter fields, numeric arguments and color grammar facts."""
import argparse,gzip,json,os,subprocess,struct
from pathlib import Path
from export_regular_clicks import ROOT,VERSIONS,digest,encoded,original_modern_classpath
from export_item_properties import COMPILER

def string(value):
    units=value.encode('utf-16-be',errors='surrogatepass');wire=bytearray()
    for i in range(0,len(units),2):
        c=int.from_bytes(units[i:i+2],'big')
        if 0<c<128:wire.append(c)
        elif c<2048:wire.extend([0xc0|(c>>6),0x80|(c&63)])
        else:wire.extend([0xe0|(c>>12),0x80|((c>>6)&63),0x80|(c&63)])
    return struct.pack('>H',len(wire))+wire

def tag(value):
    if isinstance(value,str):return 8,string(value)
    if isinstance(value,int):return (3,struct.pack('>i',value)) if -(2**31)<=value<2**31 else (4,struct.pack('>q',value))
    if isinstance(value,tuple):return value[0],value[1]
    if isinstance(value,float):return 6,struct.pack('>d',value)
    if isinstance(value,list):
        parts=[tag(v) for v in value];assert parts and len({k for k,v in parts})==1
        return 9,bytes([parts[0][0]])+struct.pack('>i',len(parts))+b''.join(v for k,v in parts)
    if isinstance(value,dict):return 10,b''.join(bytes([tag(v)[0]])+string(k)+tag(v)[1] for k,v in value.items())+b'\0'
    raise ValueError(type(value))

def requests():
    baseline=ROOT/'data/client_api/component_normalization_cases-1.21.11.json.gz'
    rows=[{'case':r['case'],'input_hex':r['input_hex']} for r in json.loads(gzip.decompress(baseline.read_bytes()))['text'] if 'input_hex' in r]
    probes=[]
    for color in ['dark_blue','dark_green','dark_aqua','dark_red','dark_purple','gold','gray','dark_gray','blue','green','aqua','light_purple','yellow', '#１２３ＡＢＣ','#٠١٢٣٤٥','#+000000ff','#000000000000000000000000000','#+','#80000000']:
        probes.append(('color-extra-'+color,{'text':'Voxrig','color':color}))
    for value in [[1.,.5,0.,1.],[-.01,0.,0.,1.],[1.01,0.,0.,1.],[0.,0.,0.,float('nan')],[0.,0.,0.,float('inf')],[0.,0.,0.,-.01]]:
        probes.append(('shadow-extra-'+str(value),{'text':'Voxrig','shadow_color':value}))
    for value in [-257,-256,-1,0,1,128,255,256,257,1.9,-.1,float('nan'),float('inf'),float('-inf')]:
        probes.append(('bold-extra-'+str(value),{'text':'Voxrig','bold':value}))
    for name,fields in [('translate-type',{'type':'translatable','translate':'voxrig.message'}),('translate-styled-arg',{'translate':'voxrig.message','with':[{'text':'arg','bold':0}]}),('translate-literal-arg',{'translate':'voxrig.message','with':[{'text':'arg'}]}),('conflict-text-keybind',{'text':'Voxrig','keybind':'key.jump'}),('conflict-keybind-text',{'keybind':'key.jump','text':'Voxrig'}),('nbt-bad-interpret',{'nbt':'value','storage':'data','interpret':'bad'}),('selector-bad-separator',{'selector':'@a','separator':{}}),('sprite-default-atlas',{'sprite':'block/stone'})]:probes.append((name,fields))
    for name,text in [('japanese','日本語'),('nul','a\0b'),('emoji','💎'),('surrogate','\ud800')]:probes.append(('literal-'+name,text))
    for kind,patterns in [(1,[b'\0',b'\xff']),(2,[b'\0\1',b'\x80\0']),(3,[struct.pack('>i',1),struct.pack('>i',-(2**31))]),(4,[struct.pack('>q',1),struct.pack('>q',2**63-1)]),(5,[struct.pack('>I',v) for v in [0,0x80000000,0x7fc00000,0x7fc00001,0x7f800000]]),(6,[struct.pack('>Q',v) for v in [0,0x8000000000000000,0x7ff8000000000000,0x7ff8000000000001,0x7ff0000000000000]])]:
        for pattern in patterns:probes.append((f'translate-number-{kind}-{pattern.hex()}',{'translate':'voxrig.message','with':[(kind,pattern)]}))
    for name,fields in probes:
        kind,value=tag(fields);rows.append({'case':name,'input_hex':(bytes([kind])+value).hex()})
    return rows

def scalar_json(value):
    if isinstance(value,str):
        try:value.encode('utf-8');return value
        except UnicodeEncodeError:
            units=value.encode('utf-16-be',errors='surrogatepass');return {'utf16':[int.from_bytes(units[i:i+2],'big') for i in range(0,len(units),2)]}
    if isinstance(value,list):return [scalar_json(v) for v in value]
    if isinstance(value,dict):return {k:scalar_json(v) for k,v in value.items()}
    return value

def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--downloads',type=Path,required=True);p.add_argument('--modern-classpath-file',type=Path,required=True);p.add_argument('--runtime-output',type=Path,required=True);p.add_argument('--normalize-only',action='store_true');p.add_argument('--check',action='store_true');a=p.parse_args()
    b=a.runtime_output.resolve();b.mkdir(parents=True,exist_ok=True);jar=(a.downloads/'1.21.11-server.jar').resolve();mapping=a.downloads/'1.21.11-server-mappings.txt';sha1,msha=VERSIONS['1.21.11']
    if digest(jar.read_bytes(),'sha1')!=sha1 or digest(mapping.read_bytes())!=msha:raise SystemExit('original inputs differ')
    cp=os.pathsep.join(str(Path(s).resolve()) for s in a.modern_classpath_file.read_text().strip().split(os.pathsep));classpath=original_modern_classpath(jar,cp)
    sources=['scripts/ExportInventoryTransfers.java','scripts/ExportItemComponents.java','scripts/ExportItemProperties.java','scripts/ExportItemComponentSchema.java','scripts/ExportComponentValueRules.java','scripts/ExportNbtSemantics.java','scripts/ExportComponentNormalization.java','scripts/ExportTextCore.java']
    named=b/'named-sources';named.mkdir(exist_ok=True)
    for s in sources:(named/Path(s).name).write_text('package voxrig.oracle;\n'+(ROOT/s).read_text())
    compiler=b/'CompileOwnTool.java';compiler.write_text(COMPILER);classes=b/'own-classes';inputs=b/'inputs.json';raw=b/'raw.json';request_bytes=encoded(requests())
    if not a.normalize_only:
        inputs.write_bytes(request_bytes)
        for label,args in [('compile',[str(compiler),cp,str(classes),*(str(named/Path(s).name) for s in sources)]),('run',['-cp',str(classes)+os.pathsep+cp,'voxrig.oracle.ExportTextCore',str(inputs),str(raw)])]:
            with (b/(label+'.log')).open('w') as log:subprocess.run(['java','-Xmx512M','-XX:ActiveProcessorCount=1',*args],cwd=b,stdout=log,stderr=subprocess.STDOUT,check=True)
    assert inputs.read_bytes()==request_bytes
    data=scalar_json(json.loads(raw.read_text()));assert len(data['colors'])==16
    colors_file='data/client_api/text_color_rules-1.21.11.json';color_bytes=encoded({'colors':data['colors'],'hex_utf16_digits':data['hex_utf16_digits']})
    case_file='data/client_api/text_core_cases-1.21.11.json.gz';case_bytes=gzip.compress(encoded({'schema':1,'cases':data['cases'],'pairs':data['pairs']},compact=True),mtime=0)
    source=encoded({'schema':1,'authority':'Unchanged original modern text stream decode and original contents/style getters/fields; original TextColor named map and running JDK Character.digit(char,16). Full complex constructor/reference/persistent/cache semantics remain incomplete. These facts are not live gameplay or inventory authority.','original_server_jar_sha1':sha1,'mappings_sha256':msha,'original_classpath_entries_sha256':classpath,'java_version':data['java_version'],'generators_sha256':{s:digest((ROOT/s).read_bytes()) for s in sources+['scripts/export_text_core.py','scripts/export_item_properties.py','scripts/export_regular_clicks.py','data/client_api/component_normalization_cases-1.21.11.json.gz']},'generated_compiler_sha256':digest(COMPILER.encode()),'requests_sha256':digest(request_bytes),'raw_output_sha256':digest(raw.read_bytes()),'local_original_numeric_inspection_sha256':digest((b/'original-text-numeric-bytecode.log').read_bytes()),'files_sha256':{colors_file:digest(color_bytes),case_file:digest(case_bytes)},'cases':len(data['cases']),'accepted':sum(c['accepted'] for c in data['cases']),'colors':16,'hex_utf16_digits':len(data['hex_utf16_digits'])})
    for name,content in [(colors_file,color_bytes),(case_file,case_bytes),('data/client_api/text_core_source.json',source)]:
        path=ROOT/name
        if a.check:
            if path.read_bytes()!=content:raise SystemExit('native text facts differ: '+name)
        else:path.write_bytes(content)
    print('Original text core facts verified' if a.check else 'Original text core facts generated',len(data['cases']),sum(c['accepted'] for c in data['cases']))
if __name__=='__main__':main()
