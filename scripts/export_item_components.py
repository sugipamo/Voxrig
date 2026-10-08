#!/usr/bin/env python3
"""Pin original 1.21.11 component registry, removal and item/packet codec facts.
No native game class is changed. One JVM at a time, 512 MiB and one CPU.
"""
import argparse
import json
import os
import subprocess
from pathlib import Path
from export_regular_clicks import ROOT, VERSIONS, digest, encoded, original_modern_classpath

COMPILER = '''import javax.tools.ToolProvider;
public class CompileOwnTool {
 public static void main(String[] a) {
  int r=ToolProvider.getSystemJavaCompiler().run(null,null,null,
    "-proc:none","-cp",a[0],"-d",a[1],a[2],a[3]);
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
    base = args.runtime_output.resolve()
    base.mkdir(parents=True, exist_ok=True)
    version = '1.21.11'
    jar_sha1, mapping_sha256 = VERSIONS[version]
    jar = (args.downloads / f'{version}-server.jar').resolve()
    mapping = args.downloads / f'{version}-server-mappings.txt'
    if digest(jar.read_bytes(), 'sha1') != jar_sha1 or digest(mapping.read_bytes()) != mapping_sha256:
        raise SystemExit('original native inputs differ')
    cp = os.pathsep.join(str(Path(p).resolve()) for p in args.modern_classpath_file.read_text().strip().split(os.pathsep))
    classpath_hashes = original_modern_classpath(jar, cp)
    sources = ['scripts/ExportInventoryTransfers.java', 'scripts/ExportItemComponents.java']
    requests = 'data/client_api/item_component_requests-1.21.11.json'
    named = base / 'named-sources'
    named.mkdir(exist_ok=True)
    for file in sources:
        (named / Path(file).name).write_text('package voxrig.oracle;\n' + (ROOT / file).read_text())
    compiler = base / 'CompileOwnTool.java'
    compiler.write_text(COMPILER)
    raw_path = base / 'raw.json'
    if not args.normalize_only:
        classes = base / 'own-classes'
        commands = [
            ('compile', [str(compiler), cp, str(classes), *(str(named / Path(p).name) for p in sources)]),
            ('run', ['-cp', str(classes) + os.pathsep + cp, 'voxrig.oracle.ExportItemComponents',
                     str(ROOT / requests), str(raw_path), version]),
        ]
        for name, command in commands:
            with (base / f'{name}.log').open('w') as log:
                subprocess.run(['java', '-Xmx512M', '-XX:ActiveProcessorCount=1', *command],
                               cwd=base, stdout=log, stderr=subprocess.STDOUT, check=True)
    raw = json.loads(raw_path.read_text())
    if raw['version'] != version or len(raw['registry']) != 104 or len(raw['removed']) != 104:
        raise SystemExit('native component registry/removal coverage differs')
    outputs = {
        'data/client_api/item_components-1.21.11.json': encoded(raw['registry']),
        'data/client_api/item_component_cases-1.21.11.json': encoded(raw),
    }
    outputs['data/client_api/item_component_source.json'] = encoded({
        'schema': 1,
        'authority': 'Untouched original native DataComponentType registry, component/patch/item and four clientbound packet stream codecs.',
        'scope': 'All 104 original component value compositions exercised by persistent requests, original item prototypes and native enum samples. Original vanilla resource/registry/tag loaders establish the oracle buffer context; each codec decoded and reencoded exactly. Default fixture registry IDs are not bindings for an arbitrary live connection. Not live receipt, semantic normalization or gameplay parity proof.',
        'vanilla_pack_selection': ['vanilla'],
        'registry_context': 'Original trusted vanilla pack repository, ResourceManager, RegistryDataLoader WORLDGEN_REGISTRIES, original tag loading/binding, frozen original registries. No fabricated holder IDs or substituted game methods.',
        'registry_binding_fixture_count': len(raw.get('vanilla_registry_bindings', {})),
        'original_server_jar_sha1': jar_sha1,
        'mappings_sha256': mapping_sha256,
        'original_classpath_entries_sha256': classpath_hashes,
        'generators_sha256': {p: digest((ROOT / p).read_bytes()) for p in sources +
                              ['scripts/export_item_components.py', 'scripts/export_regular_clicks.py', requests]},
        'generated_compiler_sha256': digest(COMPILER.encode()),
        'raw_output_sha256': digest(raw_path.read_bytes()),
        'files_sha256': {p: digest(data) for p, data in outputs.items()},
        'component_types': len(raw['registry']),
        'removal_roundtrips': len(raw['removed']),
        'component_roundtrips': len(raw['samples']),
        'item_roundtrips': len(raw['stacks']),
        'packet_roundtrips': sum(len(s['packets']) for s in raw['stacks']),
        'mixed_patch_roundtrips': 1,
        'failed_requests': raw['failures'],
    })
    for file, data in outputs.items():
        path = ROOT / file
        if args.check:
            if path.read_bytes() != data:
                raise SystemExit('native component evidence differs: ' + file)
        else:
            path.write_bytes(data)
    print('Original component evidence verified' if args.check else 'Original component evidence generated')
    print('types/removals/samples/stacks/failures:', len(raw['registry']), len(raw['removed']),
          len(raw['samples']), len(raw['stacks']), len(raw['failures']))

if __name__ == '__main__':
    main()
