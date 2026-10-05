#!/usr/bin/env python3
"""Observe unchanged original ItemStack effective fields for all bound item/prototype inputs."""
import argparse
import gzip
import json
import os
import shutil
import subprocess
from pathlib import Path
from export_regular_clicks import ROOT, VERSIONS, digest, encoded, original_modern_classpath
from export_item_properties import COMPILER
from export_text_core import scalar_json


def requests():
    native = json.loads(gzip.decompress((ROOT / 'data/client_api/item_semantics-1.21.11.json.gz').read_bytes()))
    wires = {row['input_hex'] for row in native['items']}
    for row in native['prototype_cases']:
        wires.update((row['base_hex'], row['changed_hex']))
    # Exercise replacing/adding values from every component codec, not only
    # equal-to-prototype changes. Native stream construction is the authority.
    def varint(value):
        out = bytearray()
        while value > 127:
            out.append((value & 127) | 128)
            value >>= 7
        out.append(value)
        return bytes(out)
    prototypes = json.loads((ROOT / 'data/client_api/item_properties-1.21.11.json').read_text())
    stone = next(row['native_id'] for row in prototypes['defaults'] if row['name'] == 'minecraft:stone')
    prefix = varint(1) + varint(stone) + b'\x01\x00'
    for row in native['components']:
        wires.add((prefix + varint(row['native_id']) + bytes.fromhex(row['input_hex'])).hex())
    components = json.loads((ROOT / 'data/client_api/item_component_cases-1.21.11.json').read_text())
    wires.update(row['stack_hex'] for row in components['stacks'])
    return sorted(wires)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ['downloads', 'modern-classpath-file', 'runtime-output']:
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--normalize-only', action='store_true')
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    runtime = args.runtime_output.resolve()
    runtime.mkdir(parents=True, exist_ok=True)
    jar = (args.downloads / '1.21.11-server.jar').resolve()
    mapping = args.downloads / '1.21.11-server-mappings.txt'
    sha1, mapping_hash = VERSIONS['1.21.11']
    if digest(jar.read_bytes(), 'sha1') != sha1 or digest(mapping.read_bytes()) != mapping_hash:
        raise SystemExit('original inputs differ')
    cp = os.pathsep.join(str(Path(path).resolve()) for path in args.modern_classpath_file.read_text().strip().split(os.pathsep))
    classpath = original_modern_classpath(jar, cp)
    sources = ['scripts/' + name + '.java' for name in [
        'ExportInventoryTransfers', 'ExportItemComponents', 'ExportItemProperties',
        'ExportItemComponentSchema', 'ExportComponentValueRules', 'ExportNbtSemantics',
        'ExportComponentNormalization', 'ExportTextCore', 'ExportEnchantmentConstructors',
        'ExportEffectiveItemComponents',
    ]]
    named = runtime / 'named-sources'
    named.mkdir(exist_ok=True)
    for path in sources:
        (named / Path(path).name).write_text('package voxrig.oracle;\n' + (ROOT / path).read_text())
    compiler = runtime / 'CompileOwnTool.java'
    compiler.write_text(COMPILER)
    classes = runtime / 'own-classes'
    inputs, raw = runtime / 'inputs.json', runtime / 'raw.json'
    request_bytes = encoded(requests(), compact=True)
    if not args.normalize_only:
        inputs.write_bytes(request_bytes)
        for label, command in [
            ('compile', [str(compiler), cp, str(classes), *(str(named / Path(path).name) for path in sources)]),
            ('run', ['-cp', str(classes) + os.pathsep + cp, 'voxrig.oracle.ExportEffectiveItemComponents', str(inputs), str(raw)]),
        ]:
            with (runtime / (label + '.log')).open('w') as log:
                subprocess.run(['java', '-Xmx512M', '-XX:ActiveProcessorCount=1', *command], cwd=runtime, stdout=log, stderr=subprocess.STDOUT, check=True)
    assert inputs.read_bytes() == request_bytes
    data = scalar_json(json.loads(raw.read_text()))
    case_path = 'data/client_api/effective_item_component_cases-1.21.11.json.gz'
    case_bytes = gzip.compress(encoded({'schema': 1, 'cases': data['cases'], 'values': data['values']}, compact=True), mtime=0)
    java = Path(shutil.which('java')).resolve()
    dependencies = sources + ['scripts/export_effective_item_components.py', 'scripts/export_text_core.py', 'scripts/export_item_properties.py', 'scripts/export_regular_clicks.py', 'data/client_api/item_semantics-1.21.11.json.gz', 'data/client_api/item_properties-1.21.11.json', 'data/client_api/item_component_cases-1.21.11.json']
    source = encoded({
        'schema': 1,
        'authority': 'Unchanged original ItemStack stream decode, effective component iterator, count/empty getters and one-time original component stream encoding, under observed vanilla registry context. Tests field construction, not whole item equality, live registry binding, cache or slot admission.',
        'original_server_jar_sha1': sha1,
        'mappings_sha256': mapping_hash,
        'original_classpath_entries_sha256': classpath,
        'java_version': data['java_version'],
        'java_executable_sha256': digest(java.read_bytes()),
        'jdk_modules_sha256': digest((java.parent.parent / 'lib/modules').read_bytes()),
        'generators_sha256': {path: digest((ROOT / path).read_bytes()) for path in dependencies},
        'generated_compiler_sha256': digest(COMPILER.encode()),
        'requests_sha256': digest(request_bytes),
        'raw_output_sha256': digest(raw.read_bytes()),
        'files_sha256': {case_path: digest(case_bytes)},
        'cases': len(data['cases']),
        'values': len(data['values']),
    })
    for name, content in [(case_path, case_bytes), ('data/client_api/effective_item_component_source.json', source)]:
        path = ROOT / name
        if args.check:
            if path.read_bytes() != content:
                raise SystemExit('native effective field facts differ: ' + name)
        else:
            path.write_bytes(content)
    print('Original effective item fields verified' if args.check else 'Original effective item fields generated', len(data['cases']), len(data['values']))


if __name__ == '__main__':
    main()
