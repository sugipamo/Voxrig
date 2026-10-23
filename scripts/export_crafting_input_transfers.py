#!/usr/bin/env python3
"""Observe original player/table input QUICK_MOVE in both versions/modes.

Original JARs stay local. Each JVM uses 512 MiB and one CPU, sequentially.
"""
import argparse
import gzip
import json
import os
import subprocess
from pathlib import Path
from export_regular_clicks import ROOT, VERSIONS, digest, encoded, original_modern_classpath
from export_item_properties import COMPILER


def requests(version):
    return [dict(menu=menu, slot=source, mode=mode, fixture=fixture,
                 item='minecraft:' + item, data=data, held=held)
            for menu, end in [('minecraft:player', 5), ('minecraft:crafting', 10)]
            for source in range(1, end) for mode in ['survival', 'creative']
            for fixture in ['empty', 'merge-late', 'partial', 'split', 'blocked', 'unequal']
            for item in ['stone', 'egg', 'diamond_helmet', 'carved_pumpkin']
            for data in [False, True] for held in [False, True]]


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
        'ExportComponentNormalization', 'ExportTextCore', 'ExportCraftingReturns', 'ExportCraftingInputTransfers',
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
                                'voxrig.oracle.ExportCraftingInputTransfers', version, str(inputs), str(raw)],
                               cwd=base, stdout=log, stderr=subprocess.STDOUT, check=True)
        assert inputs.read_bytes() == request_bytes
        data = json.loads(raw.read_text())
        assert data['version'] == version and len(data['cases']) == len(requests(version))
        path = 'data/client_api/crafting_input_transfer_cases-' + version + '.json.gz'
        outputs[path] = gzip.compress(encoded(data, compact=True), mtime=0)
        runs.append({'version': version, 'original_server_jar_sha1': sha1, 'mappings_sha256': mapping_hash,
                     'original_classpath_entries_sha256': classpath, 'requests_sha256': digest(request_bytes),
                     'raw_output_sha256': digest(raw.read_bytes()), 'cases': len(data['cases']),
                     'files_sha256': {path: digest(outputs[path])}})
    source = 'data/client_api/crafting_input_transfer_source.json'
    outputs[source] = encoded({'schema': 1,
                              'authority': 'Unchanged original clicked QUICK_MOVE for all player/table inputs with actual stack constructors and creative getter.',
                              'scope': 'Both versions and modes, every player/table input, forward main/hotbar routes, merge before empty, split/partial/blocked/unequal data, 64/16/1 capacities, equippable items and held data cursors. Original full slots and cursor wire values. Legacy client-side guard and modern empty native recipes/unsent native connection bound recipe callbacks; network/recalculation/cancellation need separate validation.',
                              'generators_sha256': {path: digest((ROOT / path).read_bytes()) for path in sources + ['scripts/export_crafting_input_transfers.py', 'scripts/export_regular_clicks.py']},
                              'generated_compiler_sha256': digest(COMPILER.encode()), 'runs': runs})
    for name, content in outputs.items():
        target = ROOT / name
        if args.check:
            assert target.read_bytes() == content, name
        else:
            target.write_bytes(content)
    print('original crafting_input transfers verified' if args.check else 'original crafting_input transfers generated', [(r['version'], r['cases']) for r in runs])


if __name__ == '__main__':
    main()
