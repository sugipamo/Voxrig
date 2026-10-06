#!/usr/bin/env python3
"""Observe original 1.21.11 recipe/slot display constructors and wire codecs."""
import argparse,gzip,json,os,subprocess
from pathlib import Path
from export_regular_clicks import ROOT,VERSIONS,digest,encoded,original_modern_classpath
from export_item_properties import COMPILER

def main():
 p=argparse.ArgumentParser(description=__doc__)
 for name in ['downloads','modern-classpath-file','runtime-output']:p.add_argument('--'+name,type=Path,required=True)
 p.add_argument('--normalize-only',action='store_true');p.add_argument('--check',action='store_true');a=p.parse_args()
 b=a.runtime_output.resolve();b.mkdir(parents=True,exist_ok=True)
 cp=os.pathsep.join(str(Path(v).resolve()) for v in a.modern_classpath_file.read_text().strip().split(os.pathsep))
 jar=(a.downloads/'1.21.11-server.jar').resolve();mapping=a.downloads/'1.21.11-server-mappings.txt';jarsha,mapsha=VERSIONS['1.21.11']
 assert digest(jar.read_bytes(),'sha1')==jarsha and digest(mapping.read_bytes())==mapsha
 sources=['scripts/'+n+'.java' for n in ['ExportInventoryTransfers','ExportItemComponents','ExportItemProperties','ExportRecipeDisplays']]
 named=b/'named-sources';named.mkdir(exist_ok=True)
 for s in sources:(named/Path(s).name).write_text('package voxrig.oracle;\n'+(ROOT/s).read_text())
 compiler=b/'CompileOwnTool.java';compiler.write_text(COMPILER);classes=b/'own-classes';requests='data/client_api/recipe_display_requests.json';raw=b/'raw.json'
 if not a.normalize_only:
  with (b/'compile.log').open('w') as log:subprocess.run(['java','-Xmx512M','-XX:ActiveProcessorCount=1',str(compiler),cp,str(classes),*(str(named/Path(s).name) for s in sources)],cwd=b,stdout=log,stderr=subprocess.STDOUT,check=True)
  with (b/'run.log').open('w') as log:subprocess.run(['java','-Xmx512M','-XX:ActiveProcessorCount=1','-cp',str(classes)+os.pathsep+cp,'voxrig.oracle.ExportRecipeDisplays',str(ROOT/requests),str(raw)],cwd=b,stdout=log,stderr=subprocess.STDOUT,check=True)
 data=json.loads(raw.read_text());assert data['version']=='1.21.11'
 name='data/client_api/recipe_display_cases-1.21.11.json.gz';outputs={name:gzip.compress(encoded(data,compact=True),mtime=0)}
 manifest={'schema':1,'authority':'Unmodified original native display constructors, JSON codecs and stream encode/decode, native slot/recipe/category type registries and RecipeDisplayEntry/RecipeBookAdd packet composition.','scope':'Display/packet facts only; actual recipe unlocking, recipe planning/placement, ingredients/consumption/remainders and live receipts require separate implementation and live verification.','generators_sha256':{s:digest((ROOT/s).read_bytes()) for s in sources+['scripts/export_recipe_displays.py']},'requests_sha256':{requests:digest((ROOT/requests).read_bytes())},'generated_compiler_sha256':digest(COMPILER.encode()),'runs':[{'version':'1.21.11','original_server_jar_sha1':jarsha,'mappings_sha256':mapsha,'original_classpath_entries_sha256':original_modern_classpath(jar,cp),'raw_output_sha256':digest(raw.read_bytes()),'files_sha256':{name:digest(outputs[name])}}]}
 outputs['data/client_api/recipe_display_source.json']=encoded(manifest)
 for n,v in outputs.items():
  t=ROOT/n
  if a.check:assert t.read_bytes()==v,n
  else:t.write_bytes(v)
 print('original recipe displays verified' if a.check else 'original recipe displays generated',len(data['cases']))
if __name__=='__main__':main()
