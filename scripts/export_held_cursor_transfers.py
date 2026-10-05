#!/usr/bin/env python3
"""Observe original held_cursor pickup and QUICK_MOVE in both supported versions/modes.

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


def varint(value):
    out = bytearray()
    while value > 127:
        out.append((value & 127) | 128)
        value >>= 7
    out.append(value)
    return bytes(out)


def requests(version):
    defaults = json.loads((ROOT / ('data/client_api/item_properties-' + version + '.json')).read_text())['defaults']
    def plain(name, count):
        item = next(row for row in defaults if row['name'] == 'minecraft:' + name)
        if version == '1.16.1':
            return (b'\x01' + varint(item['native_id']) + bytes([count]) + b'\0').hex()
        return (varint(count) + varint(item['native_id']) + b'\0\0').hex()
    with gzip.open(ROOT / ('data/client_api/armor_transfer_cases-' + version + '.json.gz')) as f:
        armor = json.load(f)
    labels = ['short-one', 'unbreaking'] if version == '1.16.1' else ['minecraft:enchantments-minecraft:binding_curse-1', 'minecraft:enchantments-minecraft:unbreaking-1']
    variants = [('empty', '00'), ('stone', plain('stone', 3)), ('dirt', plain('dirt', 3)), ('helmet', next(row['encoded_item_hex'] for row in defaults if row['name'] == 'minecraft:diamond_helmet'))]
    for label in labels:
        case = next(c for c in armor['cases'] if c['request']['case'] == label and c['request']['slot'] == 5 and c['request']['mode'] == 'survival')
        variants.append((label, case['before'][5]))
    menus = [('minecraft:player', 9, 36)] + [('minecraft:generic_9x' + str(n), 0, n*9) for n in range(1,7)] + [('minecraft:generic_3x3',0,9), ('minecraft:hopper',0,5), ('minecraft:shulker_box',0,27)]
    return [{'menu': menu, 'slot': source, 'mode': mode, 'fixture': fixture, 'cursor': label, 'cursor_hex': wire}
            for menu, a, b in menus for source in [a,b] for mode in ['survival','creative']
            for fixture in ['empty','merge','blocked'] for label,wire in variants]


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
        'ExportComponentNormalization', 'ExportTextCore', 'ExportEnchantmentConstructors', 'ExportHeldCursorTransfers',
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
                                'voxrig.oracle.ExportHeldCursorTransfers', version, str(inputs), str(raw)],
                               cwd=base, stdout=log, stderr=subprocess.STDOUT, check=True)
        assert inputs.read_bytes() == request_bytes
        data = json.loads(raw.read_text())
        assert data['version'] == version and len(data['cases']) == len(requests(version))
        path = 'data/client_api/held_cursor_transfer_cases-' + version + '.json.gz'
        outputs[path] = gzip.compress(encoded(data, compact=True), mtime=0)
        runs.append({'version': version, 'original_server_jar_sha1': sha1, 'mappings_sha256': mapping_hash,
                     'original_classpath_entries_sha256': classpath, 'requests_sha256': digest(request_bytes),
                     'raw_output_sha256': digest(raw.read_bytes()), 'cases': len(data['cases']),
                     'files_sha256': {path: digest(outputs[path])}})
    source = 'data/client_api/held_cursor_transfer_source.json'
    outputs[source] = encoded({'schema': 1,
                              'authority': 'Unchanged original clicked QUICK_MOVE with actual received-wire cursor constructors and actual creative getter.',
                              'scope': 'Both versions and modes, all ten supported player/storage menus, source directions, empty/partial merge/blocked destinations and empty/plain/data-bearing held cursors. Original before/after full slots and cursor wire values. Primitive context only; live reception/cancellation require separate validation.',
                              'generators_sha256': {path: digest((ROOT / path).read_bytes()) for path in sources + ['scripts/export_held_cursor_transfers.py', 'scripts/export_regular_clicks.py']},
                              'generated_compiler_sha256': digest(COMPILER.encode()), 'runs': runs})
    for name, content in outputs.items():
        target = ROOT / name
        if args.check:
            assert target.read_bytes() == content, name
        else:
            target.write_bytes(content)
    print('original held_cursor transfers verified' if args.check else 'original held_cursor transfers generated', [(r['version'], r['cases']) for r in runs])


if __name__ == '__main__':
    main()
