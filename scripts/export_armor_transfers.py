#!/usr/bin/env python3
"""Observe original armor pickup and QUICK_MOVE in both supported versions/modes.

Original JARs stay local. Each JVM uses 512 MiB and one CPU, sequentially.
"""
import argparse
import gzip
import json
import os
import struct
import subprocess
from pathlib import Path
from export_regular_clicks import ROOT, VERSIONS, digest, encoded, original_modern_classpath
from export_item_properties import COMPILER
from export_text_core import tag


def varint(value):
    out = bytearray()
    while value > 127:
        out.append((value & 127) | 128)
        value >>= 7
    out.append(value)
    return bytes(out)


def requests(version):
    variants = []
    if version == '1.16.1':
        numeric = [(name, (kind, value)) for name, kind, value in [
            ('byte-one', 1, b'\x01'), ('short-one', 2, b'\0\1'),
            ('zero', 3, struct.pack('>i', 0)), ('negative', 3, struct.pack('>i', -1)),
            ('high', 3, struct.pack('>i', 256)), ('long-wrap-zero', 4, struct.pack('>q', 1 << 32)),
            ('long-wrap-one', 4, struct.pack('>q', (1 << 32) + 1)),
            ('fraction', 5, struct.pack('>f', .99)), ('one-fraction', 6, struct.pack('>d', 1.99)),
            ('nan', 6, struct.pack('>d', float('nan'))),
            ('positive-infinity', 6, struct.pack('>d', float('inf'))),
            ('negative-infinity', 6, struct.pack('>d', -float('inf'))),
        ]]
        for name, level in numeric:
            variants.append((name, [{'id': 'minecraft:binding_curse', 'lvl': level}]))
        for name in ['binding_curse', ':binding_curse', 'minecraft:Binding_curse', 'other:binding_curse', 'minecraft:binding_curse:extra']:
            variants.append(('id-' + name, [{'id': name, 'lvl': 1}]))
        variants += [('plain', None), ('string-level', [{'id': 'minecraft:binding_curse', 'lvl': '1'}]),
                     ('missing-level', [{'id': 'minecraft:binding_curse'}]),
                     ('duplicate-zero-first', [{'id': 'binding_curse', 'lvl': 0}, {'id': 'binding_curse', 'lvl': 1}]),
                     ('duplicate-one-first', [{'id': 'binding_curse', 'lvl': 1}, {'id': 'binding_curse', 'lvl': 0}]),
                     ('unbreaking', [{'id': 'minecraft:unbreaking', 'lvl': 3}])]
        definitions = json.loads((ROOT / 'data/client_api/item_properties-1.16.1.json').read_text())
        item_id = next(row['native_id'] for row in definitions['defaults'] if row['name'] == 'minecraft:diamond_helmet')
    else:
        for field in ['minecraft:enchantments', 'minecraft:stored_enchantments']:
            for enchantment in ['minecraft:binding_curse', 'minecraft:unbreaking']:
                for level in [0, 1, 3]:
                    variants.append((field + '-' + enchantment + '-' + str(level), {field: {enchantment: level}, 'minecraft:damage': 7}))
        variants.append(('plain-damaged', {'minecraft:damage': 7}))
    rows = []
    for name, data in variants:
        for mode in ['survival', 'creative']:
            for source in [5, 6, 7, 8, 45, 9]:
                row = {'case': name, 'mode': mode, 'slot': source, 'item': 'minecraft:diamond_helmet'}
                if version == '1.16.1':
                    fields = {'Damage': 7, 'VoxrigArmorProbe': 991}
                    if data is not None:
                        fields['Enchantments'] = data
                    kind, payload = tag(fields)
                    row['input_hex'] = (b'\x01' + varint(item_id) + b'\x01' + bytes([kind]) + b'\0\0' + payload).hex()
                else:
                    row['components'] = data
                rows.append(row)
    return rows


def main():
    p = argparse.ArgumentParser(description=__doc__)
    for name in ['downloads', 'modern-classpath-file', 'runtime-output']:
        p.add_argument('--' + name, type=Path, required=True)
    p.add_argument('--normalize-only', action='store_true')
    p.add_argument('--check', action='store_true')
    args = p.parse_args()
    base = args.runtime_output.resolve()
    base.mkdir(parents=True, exist_ok=True)
    cp = os.pathsep.join(str(Path(path).resolve()) for path in args.modern_classpath_file.read_text().strip().split(os.pathsep))
    sources = ['scripts/' + name + '.java' for name in [
        'ExportInventoryTransfers', 'ExportItemComponents', 'ExportItemProperties',
        'ExportItemComponentSchema', 'ExportComponentValueRules', 'ExportNbtSemantics',
        'ExportComponentNormalization', 'ExportTextCore', 'ExportEnchantmentConstructors', 'ExportArmorTransfers',
    ]]
    named = base / 'named-sources'
    named.mkdir(exist_ok=True)
    for path in sources:
        (named / Path(path).name).write_text('package voxrig.oracle;\n' + (ROOT / path).read_text())
    compiler = base / 'CompileOwnTool.java'
    compiler.write_text(COMPILER)
    classes = base / 'own-classes'
    if not args.normalize_only:
        with (base / 'compile.log').open('w') as log:
            subprocess.run(['java', '-Xmx512M', '-XX:ActiveProcessorCount=1', str(compiler), cp, str(classes),
                            *(str(named / Path(path).name) for path in sources)],
                           cwd=base, stdout=log, stderr=subprocess.STDOUT, check=True)
    outputs, runs = {}, []
    for version, (sha1, mapping_hash) in VERSIONS.items():
        jar = (args.downloads / (version + '-server.jar')).resolve()
        mapping = args.downloads / (version + '-server-mappings.txt')
        assert digest(jar.read_bytes(), 'sha1') == sha1 and digest(mapping.read_bytes()) == mapping_hash
        runtime_cp = str(jar) if version == '1.16.1' else cp
        classpath = {digest(jar.read_bytes()): jar.name} if version == '1.16.1' else original_modern_classpath(jar, cp)
        inputs, raw = base / (version + '-inputs.json'), base / (version + '-raw.json')
        request_bytes = encoded(requests(version), compact=True)
        if not args.normalize_only:
            inputs.write_bytes(request_bytes)
            with (base / (version + '-run.log')).open('w') as log:
                subprocess.run(['java', '-Xmx512M', '-XX:ActiveProcessorCount=1', '-cp', str(classes) + os.pathsep + runtime_cp,
                                'voxrig.oracle.ExportArmorTransfers', version, str(inputs), str(raw)],
                               cwd=base, stdout=log, stderr=subprocess.STDOUT, check=True)
        assert inputs.read_bytes() == request_bytes
        data = json.loads(raw.read_text())
        assert data['version'] == version and len(data['cases']) == len(requests(version))
        path = 'data/client_api/armor_transfer_cases-' + version + '.json.gz'
        outputs[path] = gzip.compress(encoded(data, compact=True), mtime=0)
        runs.append({'version': version, 'original_server_jar_sha1': sha1, 'mappings_sha256': mapping_hash,
                     'original_classpath_entries_sha256': classpath, 'requests_sha256': digest(request_bytes),
                     'raw_output_sha256': digest(raw.read_bytes()), 'cases': len(data['cases']),
                     'files_sha256': {path: digest(outputs[path])}})
    source = 'data/client_api/armor_transfer_source.json'
    outputs[source] = encoded({'schema': 1,
                              'authority': 'Unchanged original armor mayPickup and clicked QUICK_MOVE, actual creative getter, actual native enchantment stream/NBT constructors and encoded modern enchantment registry definitions.',
                              'scope': 'Both versions and modes, four armor slots, main/offhand controls, native legacy numeric getters/identifier/first duplicate semantics, modern actual/stored enchantments and levels. Primitive context only; live reception/cancellation require separate validation.',
                              'generators_sha256': {path: digest((ROOT / path).read_bytes()) for path in sources + ['scripts/export_armor_transfers.py', 'scripts/export_text_core.py']},
                              'generated_compiler_sha256': digest(COMPILER.encode()), 'runs': runs})
    for name, content in outputs.items():
        target = ROOT / name
        if args.check:
            assert target.read_bytes() == content, name
        else:
            target.write_bytes(content)
    print('original armor transfers verified' if args.check else 'original armor transfers generated', [(r['version'], r['cases']) for r in runs])


if __name__ == '__main__':
    main()
