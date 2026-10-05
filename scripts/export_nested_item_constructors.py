#!/usr/bin/env python3
"""Record unchanged native nested item/prototype/weight constructor fields and equality."""
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
    native=json.loads(gzip.decompress((ROOT/'data/client_api/item_semantics-1.21.11.json.gz').read_bytes()))
    prototypes=json.loads((ROOT/'data/client_api/item_properties-1.21.11.json').read_text())
    item_ids={r['name']:r['native_id'] for r in prototypes['defaults']}
    def varint(value):
        value &= 0xffffffff;out=bytearray()
        while value>127:out.append((value&127)|128);value >>=7
        out.append(value);return bytes(out)
    def item(name,count=1,added=(),removed=()):
        return varint(count)+varint(item_ids['minecraft:'+name])+varint(len(added))+varint(len(removed))+b''.join(varint(k)+v for k,v in added)+b''.join(varint(k) for k in removed)
    def bundle(items):return varint(len(items))+b''.join(items)
    def bee(count):return varint(count)+b''.join(b'\0\x0a\0\0\0' for _ in range(count))
    p=next(r for r in native['prototype_cases'] if r['case'].startswith('minecraft:stone-same-prototype-'))
    base=bytes.fromhex(p['base_hex']);same=bytes.fromhex(p['changed_hex']);rows=[]
    variants=[('empty-list',[]),('empty-item',[b'\0']),('two-empty',[b'\0',b'\0']),('base',[base]),('same-prototype',[same]),('base-empty',[base,b'\0']),('same-prototype-empty',[same,b'\0']),('empty-base',[b'\0',base]),('twice-base',[base,base])]
    for name,values in variants:
        for component in ['container','bundle_contents','charged_projectiles']:
            rows.append({'component':'minecraft:'+component,'case':name,'input_hex':bundle(values).hex()})
    for count in [-2147483648,-1,0,1,2,2147483647]:
        nested=varint(count) if count<=0 else varint(count)+base[1:]
        for component in ['container','bundle_contents','charged_projectiles','use_remainder']:
            wire=nested if component=='use_remainder' else bundle([nested]);rows.append({'component':'minecraft:'+component,'case':'count-'+str(count),'input_hex':wire.hex()})
    for component in ['container','bundle_contents','charged_projectiles','use_remainder']:
        for capacity in [-2147483648,-2147483647,-65536,-2,-1,0,1,2,3,16,64,65536,2147483646,2147483647]:
            for count in [1,2,2147483647]:
                nested=item('stone',count,[(1,varint(capacity))]);wire=nested if component=='use_remainder' else bundle([nested]);rows.append({'component':'minecraft:'+component,'case':f'capacity-{capacity}-count-{count}','input_hex':wire.hex()})
        for count in [1,2,2147483647]:
            nested=item('air',count);wire=nested if component=='use_remainder' else bundle([nested]);rows.append({'component':'minecraft:'+component,'case':'air-'+str(count),'input_hex':wire.hex()})
    extra=[]
    for name in ['stone','bundle','beehive','oak_boat']:
        for removed in [(),(1,),(48,),(75,),(1,48,75)]:extra.append((f'{name}-removed-{removed}',[item(name,removed=removed)]))
    for contents in [[],[base],[same],[base,base]]:
        for bees in [0,1,2]:
            for capacity in [0,1,64]:
                for order in [False,True]:
                    added=[(1,varint(capacity)),(48,bundle(contents)),(75,bee(bees))];added=added[::-1] if order else added
                    extra.append((f'routes-{len(contents)}-{bees}-{capacity}-{order}',[item('stone',added=added)]))
    for bees in [0,1,2]:
        for capacity in [-2147483648,0,1,64]:
            for count in [1,2,2147483647]:
                for reverse in [False,True]:
                    added=[(1,varint(capacity)),(75,bee(bees))];added=added[::-1] if reverse else added
                    extra.append((f'bee-only-{bees}-{capacity}-{count}-{reverse}',[item('stone',count,added=added)]))
    for count in [1,2,2147483647]:
        for removed in [(),(48,)]:extra.append((f'default-bundle-{count}-{removed}',[item('bundle',count,removed=removed)]))
    for depth in [1,2,3,4,8]:
        nested=base
        for _ in range(depth):nested=item('stone',added=[(48,bundle([nested]))])
        extra.append((f'nesting-{depth}',[nested]))
    for capacities in [[2,3],[2147483647,2147483646],[65536,65537],[-1,1],[-2,2],[3,3,3]]:
        extra.append((f'sum-{capacities}',[item('stone',added=[(1,varint(n))]) for n in capacities]))
    for name,values in extra:rows.append({'component':'minecraft:bundle_contents','case':name,'input_hex':bundle(values).hex()})
    for component,wire in [('container',bundle([base,b'\0'])),('bundle_contents',bundle([base]))]:
        for length in range(len(wire)):rows.append({'component':'minecraft:'+component,'case':'truncated-'+str(length),'input_hex':wire[:length].hex()})
        rows.append({'component':'minecraft:'+component,'case':'trailing','input_hex':(wire+b'\0').hex()})
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
    sources=['scripts/ExportInventoryTransfers.java','scripts/ExportItemComponents.java','scripts/ExportItemProperties.java','scripts/ExportItemComponentSchema.java','scripts/ExportComponentValueRules.java','scripts/ExportNbtSemantics.java','scripts/ExportComponentNormalization.java','scripts/ExportTextCore.java','scripts/ExportEnchantmentConstructors.java','scripts/ExportNestedItemConstructors.java']
    named=b/'named-sources';named.mkdir(exist_ok=True)
    for s in sources:(named/Path(s).name).write_text('package voxrig.oracle;\n'+(ROOT/s).read_text())
    compiler=b/'CompileOwnTool.java';compiler.write_text(COMPILER);classes=b/'own-classes';inputs=b/'inputs.json';raw=b/'raw.json';request_bytes=encoded(requests())
    if not a.normalize_only:
        inputs.write_bytes(request_bytes)
        for label,args in [('compile',[str(compiler),cp,str(classes),*(str(named/Path(s).name) for s in sources)]),('run',['-cp',str(classes)+os.pathsep+cp,'voxrig.oracle.ExportNestedItemConstructors',str(inputs),str(raw)])]:
            with (b/(label+'.log')).open('w') as log:subprocess.run(['java','-Xmx512M','-XX:ActiveProcessorCount=1',*args],cwd=b,stdout=log,stderr=subprocess.STDOUT,check=True)
    assert inputs.read_bytes()==request_bytes
    data=scalar_json(json.loads(raw.read_text()));rules_file='data/client_api/nested_item_constructor_rules-1.21.11.json';rules_bytes=encoded({'optional_item_count_nonpositive':'empty','nonempty_item_rejects':['nonpositive count','air'],'container_list_trailing_empty':'retained','bundle_weight_routes':['nested bundle +1/16','nonempty bees=1','otherwise1/max_stack_size'],'bundle_count':'multiplyBy count;add each result in list order','removed_max_stack_size':1,'default_bundle_bees_prototypes':'empty lists only in pinned native registry'});cases_file='data/client_api/nested_item_constructor_cases-1.21.11.json.gz';case_bytes=gzip.compress(encoded({'schema':1,'cases':data['cases'],'pairs':data['pairs']},compact=True),mtime=0)
    source=encoded({'schema':1,'authority':'Unchanged original nested item component stream codecs, ItemStack empty/count/property getters,original nested encoding/list fields,bundle Fraction getters/selected and component.equals under observed vanilla registry context. Prototype equivalent inputs from bound original corpus. No full Rust component/item equality,live registry,persistent/server cache or admission proof.','original_server_jar_sha1':sha1,'mappings_sha256':msha,'original_classpath_entries_sha256':classpath,'java_version':data['java_version'],'java_executable_sha256':digest(Path(__import__('shutil').which('java')).resolve().read_bytes()),'jdk_modules_sha256':digest((Path(__import__('shutil').which('java')).resolve().parent.parent/'lib/modules').read_bytes()),'generators_sha256':{s:digest((ROOT/s).read_bytes()) for s in sources+['scripts/export_nested_item_constructors.py','scripts/export_text_core.py','scripts/export_item_properties.py','scripts/export_regular_clicks.py','data/client_api/item_semantics-1.21.11.json.gz','data/client_api/item_properties-1.21.11.json']},'generated_compiler_sha256':digest(COMPILER.encode()),'requests_sha256':digest(request_bytes),'raw_output_sha256':digest(raw.read_bytes()),'local_original_nested_item_constructor_inspection_sha256':digest((b/'original-nested-items-bytecode.log').read_bytes()),'files_sha256':{rules_file:digest(rules_bytes),cases_file:digest(case_bytes)},'cases':len(data['cases']),'accepted':sum(c['accepted'] for c in data['cases']),'pairs':len(data['pairs'])})
    for name,content in [(rules_file,rules_bytes),(cases_file,case_bytes),('data/client_api/nested_item_constructor_source.json',source)]:
        path=ROOT/name
        if a.check:
            if path.read_bytes()!=content:raise SystemExit('native nested item constructor facts differ: '+name)
        else:path.write_bytes(content)
    print('Original nested item constructor facts verified' if a.check else 'Original nested item constructor facts generated',len(data['cases']),sum(c['accepted'] for c in data['cases']),len(data['pairs']))


if __name__=='__main__':main()
