#!/usr/bin/env python3
"""Original typed scalar/enum codec facts; separate from full component semantics."""
import argparse,gzip,json,os,subprocess
from pathlib import Path
from export_regular_clicks import ROOT,VERSIONS,digest,encoded,original_modern_classpath
from export_item_properties import COMPILER
from export_item_component_schema import normalize

def main():
 p=argparse.ArgumentParser(description=__doc__)
 p.add_argument('--downloads',type=Path,required=True);p.add_argument('--modern-classpath-file',type=Path,required=True);p.add_argument('--runtime-output',type=Path,required=True);p.add_argument('--normalize-only',action='store_true');p.add_argument('--check',action='store_true');a=p.parse_args()
 b=a.runtime_output.resolve();b.mkdir(parents=True,exist_ok=True)
 jar=(a.downloads/'1.21.11-server.jar').resolve();mapping=a.downloads/'1.21.11-server-mappings.txt';sha1,msha=VERSIONS['1.21.11']
 if digest(jar.read_bytes(),'sha1')!=sha1 or digest(mapping.read_bytes())!=msha:raise SystemExit('original inputs differ')
 cp=os.pathsep.join(str(Path(s).resolve()) for s in a.modern_classpath_file.read_text().strip().split(os.pathsep));classpath=original_modern_classpath(jar,cp)
 sources=['scripts/ExportInventoryTransfers.java','scripts/ExportItemComponents.java','scripts/ExportItemProperties.java','scripts/ExportItemComponentSchema.java','scripts/ExportComponentValueRules.java','scripts/ExportNbtSemantics.java']
 named=b/'named-sources';named.mkdir(exist_ok=True)
 for source in sources:(named/Path(source).name).write_text('package voxrig.oracle;\n'+(ROOT/source).read_text())
 compiler=b/'CompileOwnTool.java';compiler.write_text(COMPILER);classes=b/'own-classes';raw=b/'raw.json'
 if not a.normalize_only:
  for label,args in [('compile',[str(compiler),cp,str(classes),*(str(named/Path(s).name) for s in sources)]),('run',['-cp',str(classes)+os.pathsep+cp,'voxrig.oracle.ExportComponentValueRules',str(raw)])]:
   with (b/(label+'.log')).open('w') as log:subprocess.run(['java','-Xmx512M','-XX:ActiveProcessorCount=1',*args],cwd=b,stdout=log,stderr=subprocess.STDOUT,check=True)
 data=json.loads(raw.read_text());schema=json.loads((ROOT/'data/client_api/item_component_schema-1.21.11.json').read_text())
 assert normalize(data['graph'])==schema,'actual loaded-native composition differs from pinned framing schema'
 for entry in data['enums']:
  values=sorted(entry['values'],key=lambda v:v['native_id']);valid={v['native_id'] for v in values};assert len(valid)==len(values) and 0 in valid
  by_id={v['native_id']:v for v in values};n=len(values);candidates={'wrap':lambda x:x%n,'clamp':lambda x:min(max(x,0),n-1),'zero':lambda x:x if x in valid else 0}
  modes=[m for m,f in candidates.items() if all(f(r['input'])==r['native_id'] and by_id[r['native_id']]['name']==r['name'] for r in entry['probes'])]
  assert len(modes)==1,('unreviewed original enum behavior',entry['node'],modes)
  entry['normalization']=modes[0];entry['values']=values
  if modes[0]!='zero':assert sorted(valid)==list(range(n))
 file='data/client_api/component_value_rules-1.21.11.json'
 output=encoded({'schema':1,'enums':[{k:v for k,v in e.items() if k!='probes'} for e in data['enums']],'scalars':[{k:v for k,v in e.items() if k not in ['probes','canonical_hex']} for e in data['scalars']]})
 corpus_file='data/client_api/component_value_cases-1.21.11.json.gz'
 corpus=gzip.compress(encoded({'schema':1,'enums':data['enums'],'scalars':data['scalars'],'unnamed_tags':data['unnamed_tags']},compact=True),mtime=0)
 source=encoded({'schema':1,'authority':'Unchanged original enum IntFunction/ToIntFunction, scalar stream codec decoders/encoders and actual loaded native codec compositions. These are primitive normalization facts, not complete component semantic/equality/hash or gameplay support.','original_server_jar_sha1':sha1,'mappings_sha256':msha,'original_classpath_entries_sha256':classpath,'generators_sha256':{s:digest((ROOT/s).read_bytes()) for s in sources+['scripts/export_component_value_rules.py','scripts/export_item_component_schema.py','scripts/export_item_components.py','scripts/export_item_properties.py','scripts/export_regular_clicks.py','data/client_api/item_component_schema-1.21.11.json']},'generated_compiler_sha256':digest(COMPILER.encode()),'raw_output_sha256':digest(raw.read_bytes()),'local_original_by_id_map_inspection_sha256':digest((b/'by-id-map-bytecode.log').read_bytes()),'files_sha256':{file:digest(output),corpus_file:digest(corpus)},'enums':len(data['enums']),'scalar_nodes':len(data['scalars']),'unnamed_tags':len(data['unnamed_tags'])})
 for name,content in [(file,output),(corpus_file,corpus),('data/client_api/component_value_rules_source.json',source)]:
  path=ROOT/name
  if a.check:
   if path.read_bytes()!=content:raise SystemExit('original component value facts differ: '+name)
  else:path.write_bytes(content)
 print('Original component value facts verified' if a.check else 'Original component value facts generated',len(data['enums']),len(data['scalars']))
if __name__=='__main__':main()
