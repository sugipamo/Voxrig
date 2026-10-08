#!/usr/bin/env python3
"""Pin unchanged native item prototypes/property getters; one JVM, 512 MiB/CPU 1."""
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
    sources = ['scripts/ExportInventoryTransfers.java', 'scripts/ExportItemComponents.java', 'scripts/ExportItemProperties.java']
    requests = 'data/client_api/item_properties_requests.json'
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
                                'voxrig.oracle.ExportItemProperties',version,str(ROOT / requests),str(raw)],
                               cwd=base,stdout=log,stderr=subprocess.STDOUT,check=True)
        data = json.loads(raw.read_text()); assert data['version'] == version
        file = 'data/client_api/item_properties-' + version + '.json'
        outputs[file] = encoded(data)
        runs.append({'version':version,'original_server_jar_sha1':jar_sha1,'mappings_sha256':mapping_sha256,
                     'original_classpath_entries_sha256':hashes,'raw_output_sha256':digest(raw.read_bytes()),
                     'defaults':len(data['defaults']),'prototype_values':len(data['prototype_values']),
                     'cases':len(data['cases']),'original_invalid_cases':len(data['failures']),
                     'files_sha256':{file:digest(outputs[file])}})
    outputs['data/client_api/item_properties_source.json'] = encoded({
        'schema':1,'authority':'Unchanged original item constructor/prototype, full item stream decoder/encoder and native max-stack/durability/stackability getters.',
        'scope':'Native default item facts, raw prototype component values and effective property behavior including removal and untrusted stream scalar boundaries. Prototype reference bytes belong to the pinned vanilla oracle registry context, not arbitrary live registry identities. Not complete semantic equality, inventory/cache hash, slot acceptance or gameplay permission.',
        'generators_sha256':{p:digest((ROOT / p).read_bytes()) for p in sources+[requests,'scripts/export_item_properties.py','scripts/export_regular_clicks.py']},
        'generated_compiler_sha256':digest(COMPILER.encode()),'runs':runs})
    for name, data in outputs.items():
        path = ROOT / name
        if args.check:
            if path.read_bytes() != data: raise SystemExit('Native item property facts differ: ' + name)
        else: path.write_bytes(data)
    print('Original item properties verified' if args.check else 'Original item properties generated')
    for run in runs: print(run['version'],'defaults/prototype-values/cases/invalid',run['defaults'],run['prototype_values'],run['cases'],run['original_invalid_cases'])

if __name__ == '__main__': main()
