#!/usr/bin/env python3
"""Pin original builtin/networkable registry catalogs and legacy registry codec; sequential JVM512MiB/CPU1."""
import argparse
import json
import os
from pathlib import Path
import subprocess
from export_regular_clicks import ROOT, VERSIONS, digest, encoded, original_modern_classpath

COMPILER = '''import javax.tools.ToolProvider;
public class CompileOwnTool {
 public static void main(String[] a) {
  String[] opts=new String[a.length+3];
  opts[0]="-proc:none";opts[1]="-cp";opts[2]=a[0];opts[3]="-d";opts[4]=a[1];
  System.arraycopy(a,2,opts,5,a.length-2);
  int r=ToolProvider.getSystemJavaCompiler().run(null,null,null,opts);
  if(r!=0)throw new IllegalStateException("own tooling compilation failed");
 }
}
'''

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--downloads', type=Path, required=True)
    parser.add_argument('--modern-classpath-file', type=Path, required=True)
    parser.add_argument('--runtime-output', type=Path, required=True)
    parser.add_argument('--normalize-only', action='store_true')
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    base = args.runtime_output.resolve(); base.mkdir(parents=True, exist_ok=True)
    cp = os.pathsep.join(str(Path(p).resolve()) for p in args.modern_classpath_file.read_text().strip().split(os.pathsep))
    sources = ['scripts/ExportInventoryTransfers.java', 'scripts/ExportItemComponents.java', 'scripts/ExportItemProperties.java', 'scripts/ExportRegistryCatalog.java']
    named = base / 'named-sources'; named.mkdir(exist_ok=True)
    for source in sources:
        (named / Path(source).name).write_text('package voxrig.oracle;\n' + (ROOT / source).read_text())
    compiler = base / 'CompileOwnTool.java'; compiler.write_text(COMPILER)
    classes = base / 'own-classes'
    inputs = []
    for version, (jar_sha1, mapping_sha256) in VERSIONS.items():
        jar = (args.downloads / (version + '-server.jar')).resolve()
        mapping = args.downloads / (version + '-server-mappings.txt')
        if digest(jar.read_bytes(), 'sha1') != jar_sha1 or digest(mapping.read_bytes()) != mapping_sha256:
            raise SystemExit('original inputs differ: ' + version)
        hashes = original_modern_classpath(jar, cp) if version == '1.21.11' else {digest(jar.read_bytes()): jar.name}
        inputs.append((version, jar, jar_sha1, mapping_sha256, hashes))
    if not args.normalize_only:
        with (base / 'compile.log').open('w') as log:
            subprocess.run(['java','-Xmx512M','-XX:ActiveProcessorCount=1',str(compiler),cp,str(classes),
                            *(str(named / Path(s).name) for s in sources)],cwd=base,stdout=log,stderr=subprocess.STDOUT,check=True)
    outputs, runs = {}, []
    for version, jar, jar_sha1, mapping_sha256, hashes in inputs:
        raw = base / (version + '-raw.json')
        if not args.normalize_only:
            with (base / (version + '-run.log')).open('w') as log:
                subprocess.run(['java','-Xmx512M','-XX:ActiveProcessorCount=1','-cp',str(classes)+os.pathsep+(str(jar) if version=='1.16.1' else cp),
                                'voxrig.oracle.ExportRegistryCatalog',version,str(raw)],
                               cwd=base,stdout=log,stderr=subprocess.STDOUT,check=True)
        data = json.loads(raw.read_text()); assert data['version'] == version
        catalog = {'schema':1, 'version':version, 'registries':[{'name':r['name'],'builtin':r['builtin'],'networkable':r['networkable'],'entries':r['entries'] if r['builtin'] else []} for r in data['registries']]}
        file = 'data/client_api/registry_catalog-' + version + '.json'
        case_file = 'data/client_api/registry_catalog_cases-' + version + '.json'
        outputs[file] = encoded(catalog)
        outputs[case_file] = encoded(data)
        runs.append({'version':version,'original_server_jar_sha1':jar_sha1,'mappings_sha256':mapping_sha256,
                     'original_classpath_entries_sha256':hashes,'raw_output_sha256':digest(raw.read_bytes()),
                     'registry_count':len(data['registries']),'builtin_count':sum(r['builtin'] for r in data['registries']),
                     'networkable_count':sum(r['networkable'] for r in data['registries']),
                     'files_sha256':{f:digest(outputs[f]) for f in [file,case_file]}})
    java=Path(__import__('shutil').which('java')).resolve()
    outputs['data/client_api/registry_catalog_source.json'] = encoded({
        'schema':1,'authority':'Unchanged original builtin root identity, original registry entry names and native IDs, modern RegistrySynchronization networkable filter and legacy RegistryAccess builtin codec/getters.',
        'scope':'Runtime catalogs include IDs only for original builtin root entries. Dynamic vanilla IDs are test facts, not injected into received registries. No native whole component/reference/tag equality, cache, slot admission or general gameplay proof.',
        'generators_sha256':{p:digest((ROOT / p).read_bytes()) for p in sources+['scripts/export_registry_catalog.py','scripts/export_item_properties.py','scripts/export_regular_clicks.py']},
        'generated_compiler_sha256':digest(COMPILER.encode()),'java_executable_sha256':digest(java.read_bytes()),
        'jdk_modules_sha256':digest((java.parent.parent/'lib/modules').read_bytes()),'runs':runs})
    for name, data in outputs.items():
        path = ROOT / name
        if args.check:
            if path.read_bytes() != data: raise SystemExit('Native item property facts differ: ' + name)
        else: path.write_bytes(data)
    print('Original registry catalogs verified' if args.check else 'Original registry catalogs generated')
    for run in runs: print(run['version'],'registries/builtin/networkable',run['registry_count'],run['builtin_count'],run['networkable_count'])

if __name__ == '__main__': main()
