#!/usr/bin/env python3
"""Observe unchanged modern registry-aware holder constructors/lookups and equals."""
import argparse
import gzip
import json
import os
import shutil
import subprocess
from pathlib import Path
from export_regular_clicks import ROOT, VERSIONS, digest, encoded, original_modern_classpath
from export_item_properties import COMPILER

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--downloads', type=Path, required=True)
    parser.add_argument('--modern-classpath-file', type=Path, required=True)
    parser.add_argument('--runtime-output', type=Path, required=True)
    parser.add_argument('--normalize-only', action='store_true')
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    base = args.runtime_output.resolve(); base.mkdir(parents=True, exist_ok=True)
    jar = (args.downloads / '1.21.11-server.jar').resolve()
    mapping = args.downloads / '1.21.11-server-mappings.txt'
    sha1, mapping_sha = VERSIONS['1.21.11']
    if digest(jar.read_bytes(), 'sha1') != sha1 or digest(mapping.read_bytes()) != mapping_sha:
        raise SystemExit('original inputs differ')
    cp = os.pathsep.join(str(Path(p).resolve()) for p in args.modern_classpath_file.read_text().strip().split(os.pathsep))
    classpath = original_modern_classpath(jar, cp)
    sources = ['scripts/ExportInventoryTransfers.java','scripts/ExportItemComponents.java','scripts/ExportItemProperties.java','scripts/ExportHolderComparison.java']
    named = base / 'named-sources'; named.mkdir(exist_ok=True)
    for source in sources:
        (named / Path(source).name).write_text('package voxrig.oracle;\n' + (ROOT / source).read_text())
    compiler = base / 'CompileOwnTool.java'; compiler.write_text(COMPILER)
    classes = base / 'own-classes'; raw = base / 'raw.json'
    if not args.normalize_only:
        for label, arguments in [('compile', [str(compiler),cp,str(classes),*(str(named/Path(s).name) for s in sources)]), ('run',['-cp',str(classes)+os.pathsep+cp,'voxrig.oracle.ExportHolderComparison',str(raw)])]:
            with (base/(label+'.log')).open('w') as log:
                subprocess.run(['java','-Xmx512M','-XX:ActiveProcessorCount=1',*arguments],cwd=base,stdout=log,stderr=subprocess.STDOUT,check=True)
    data = json.loads(raw.read_text())
    cases_file = 'data/client_api/holder_comparison_cases-1.21.11.json.gz'
    cases = gzip.compress(encoded(data, compact=True), mtime=0)
    java = Path(shutil.which('java')).resolve()
    source = encoded({'schema':1,'authority':'Unchanged original MappedRegistry, Holder and HolderSet constructors/getters, registry-aware holder/tag stream decoding and Object.equals. Two independently registered original enchantment registries use the same original enchantment values in reversed ID order.','scope':'Constructor/lookup/comparison facts only, not a live vanilla server/cache/slot admission or full general component comparison proof.','original_server_jar_sha1':sha1,'mappings_sha256':mapping_sha,'original_classpath_entries_sha256':classpath,'java_version':data['java_version'],'java_executable_sha256':digest(java.read_bytes()),'jdk_modules_sha256':digest((java.parent.parent/'lib/modules').read_bytes()),'generators_sha256':{s:digest((ROOT/s).read_bytes()) for s in sources+['scripts/export_holder_comparison.py','scripts/export_item_properties.py','scripts/export_regular_clicks.py']},'generated_compiler_sha256':digest(COMPILER.encode()),'raw_output_sha256':digest(raw.read_bytes()),'files_sha256':{cases_file:digest(cases)},'cases':len(data['cases']),'pairs':len(data['pairs'])})
    for file, value in [(cases_file,cases),('data/client_api/holder_comparison_source.json',source)]:
        path = ROOT / file
        if args.check:
            if path.read_bytes() != value: raise SystemExit('native holder facts differ: '+file)
        else: path.write_bytes(value)
    print('Original holder comparison facts verified' if args.check else 'Original holder comparison facts generated',len(data['cases']),len(data['pairs']))

if __name__ == '__main__': main()
