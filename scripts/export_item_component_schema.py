#!/usr/bin/env python3
"""Export original component codec composition facts; no game method bodies.
Shapes describe encoded field boundaries, not registry identity or gameplay semantics.
"""
import argparse
import json
import os
import subprocess
from pathlib import Path
from export_regular_clicks import ROOT, VERSIONS, digest, encoded, original_modern_classpath
from export_item_components import COMPILER

def normalize(raw):
    nodes=[]
    composites={'aao$18','aao$19','aao$2','aao$3','aao$4','aao$5','aao$7','aao$9'}
    for n in raw['nodes']:
        cls=n['class'];f={v['name']:v for v in n['fields']}
        children=[v['codec'] for v in n['fields'] if 'codec' in v]
        op={}
        if cls in composites:op={'op':'sequence','children':children}
        elif cls in {'aao$14','aao$17','aam$10'}:op={'op':'forward','child':children[0]}
        elif cls=='aao$11':op={'op':'forward','child':n['resolved']}
        elif cls=='aam$1':op={'op':'boolean'}
        elif cls=='aam$2':op={'op':'fixed','length':8}
        elif cls=='aam$35' or cls=='aam$31':op={'op':'fixed','length':4}
        elif cls=='is$1':op={'op':'fixed','length':8}
        elif cls=='jx$1':op={'op':'fixed','length':16}
        elif cls=='aam$32' or cls=='aam$21':op={'op':'varint'}
        elif cls=='aam$8':op={'op':'nbt'}
        elif cls=='aao$13':op={'op':'unit'}
        elif cls=='aam$6':op={'op':'string','maximum':int(f['a']['value'])}
        elif cls=='aam$16':op={'op':'optional','child':children[0]}
        elif cls=='aam$17':op={'op':'list','child':children[0],'maximum':int(f['a']['value'])}
        elif cls=='aam$18':op={'op':'map','key':children[0],'value':children[1],'maximum':int(f['a']['value'])}
        elif cls=='aam$19':op={'op':'either','left':children[0],'right':children[1]}
        elif cls=='aam$22':op={'op':'registry','registry':f['b']['key'].split(' / ')[1][:-1]}
        elif cls=='aam$24':op={'op':'holder','registry':f['a']['key'].split(' / ')[1][:-1],'inline':children[0]}
        elif cls=='aam$25':op={'op':'holder_set','registry':f['a']['key'].split(' / ')[1][:-1],'child':children[0]}
        elif cls=='aam$26':op={'op':'profile_properties'}
        elif cls=='kk$1':op={'op':'typed_component'}
        elif cls=='dlt$1':op={'op':'item','nonempty':False}
        elif cls=='dlt$2':op={'op':'item','nonempty':True}
        elif cls=='kg$3':op={'op':'patch'}
        elif cls=='aao$16':
            variants=n['variants'];op={'op':'dispatch','branched':any('left' in v for v in variants),'variants':variants}
        else:raise ValueError('unreviewed original stream-codec class '+cls)
        nodes.append({'native_class':cls,**op})
    assert len(raw['roots'])==105 and len(nodes)==651
    return {'schema':1,'version':'1.21.11','roots':raw['roots'],'nodes':nodes}

def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--downloads',type=Path,required=True)
    parser.add_argument('--modern-classpath-file',type=Path,required=True)
    parser.add_argument('--runtime-output',type=Path,required=True)
    parser.add_argument('--normalize-only',action='store_true')
    parser.add_argument('--check',action='store_true')
    args=parser.parse_args();base=args.runtime_output.resolve();base.mkdir(parents=True,exist_ok=True)
    jar=(args.downloads/'1.21.11-server.jar').resolve();mapping=args.downloads/'1.21.11-server-mappings.txt'
    jar_sha1,mapping_sha256=VERSIONS['1.21.11']
    if digest(jar.read_bytes(),'sha1')!=jar_sha1 or digest(mapping.read_bytes())!=mapping_sha256:raise SystemExit('original native inputs differ')
    cp=os.pathsep.join(str(Path(s).resolve()) for s in args.modern_classpath_file.read_text().strip().split(os.pathsep))
    classpath_hashes=original_modern_classpath(jar,cp)
    sources=['scripts/ExportInventoryTransfers.java','scripts/ExportItemComponentSchema.java']
    named=base/'named-sources';named.mkdir(exist_ok=True)
    for source in sources:(named/Path(source).name).write_text('package voxrig.oracle;\n'+(ROOT/source).read_text())
    compiler=base/'CompileOwnTool.java';compiler.write_text(COMPILER)
    raw=base/'raw.json';classes=base/'own-classes'
    if not args.normalize_only:
        for label,command in [('compile',[str(compiler),cp,str(classes),*[str(named/Path(p).name) for p in sources]]),('run',['-cp',str(classes)+os.pathsep+cp,'voxrig.oracle.ExportItemComponentSchema',str(raw)])]:
            with (base/f'{label}.log').open('w') as log:subprocess.run(['java','-Xmx512M','-XX:ActiveProcessorCount=1',*command],cwd=base,stdout=log,stderr=subprocess.STDOUT,check=True)
    normalized=normalize(json.loads(raw.read_text()));file='data/client_api/item_component_schema-1.21.11.json';data=encoded(normalized)
    source={'schema':1,'authority':'Unmodified original native registry and actual stream-codec object compositions, including resolved recursion and original static dispatcher branches. Encoded-boundary facts, not method bodies or semantic/gameplay parity.',
        'original_server_jar_sha1':jar_sha1,'mappings_sha256':mapping_sha256,'original_classpath_entries_sha256':classpath_hashes,
        'generators_sha256':{p:digest((ROOT/p).read_bytes()) for p in sources+['scripts/export_item_component_schema.py','scripts/export_item_components.py','scripts/export_regular_clicks.py']},
        'generated_compiler_sha256':digest(COMPILER.encode()),'raw_output_sha256':digest(raw.read_bytes()),'files_sha256':{file:digest(data)},'component_roots':104,'codec_nodes':len(normalized['nodes'])}
    for name,content in [(file,data),('data/client_api/item_component_schema_source.json',encoded(source))]:
        p=ROOT/name
        if args.check:
            if p.read_bytes()!=content:raise SystemExit('native component schema differs: '+name)
        else:p.write_bytes(content)
    print('Original component schema verified' if args.check else 'Original component schema generated',104,len(normalized['nodes']))

if __name__=='__main__':main()
