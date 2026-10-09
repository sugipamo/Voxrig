#!/usr/bin/env python3
"""Observe modified equippable routing on the pinned official modern server JAR.

Runs one original menu primitive observer with a 512 MiB heap and one CPU.
This is not a live connection, receive, cancellation or game-mode proof.
"""
import argparse
import gzip
import json
import os
import subprocess
from pathlib import Path
from export_regular_clicks import ROOT, VERSIONS, digest, encoded, original_modern_classpath
from export_item_properties import COMPILER


def requests():
    rows = []
    for item in ['stone', 'carved_pumpkin', 'diamond_helmet', 'shield']:
        for slot in ['mainhand', 'feet', 'legs', 'chest', 'head', 'offhand', 'body', 'saddle']:
            for allowed in [None, [], ['minecraft:player'], ['minecraft:cow'], '#minecraft:can_wear_horse_armor']:
                value = {'slot': slot}
                if allowed is not None:
                    value['allowed_entities'] = allowed
                for source in [9, 36]:
                    for occupied in [False, True]:
                        # Native max capacity overrides allow partial armor transfer too.
                        rows.append({'item': 'minecraft:' + item, 'count': 3,
                                     'slot': source, 'occupied': occupied,
                                     'components': {'minecraft:equippable': value,
                                                    'minecraft:max_stack_size': 64}})
        for source in [9, 36]:
            rows.append({'item': 'minecraft:' + item, 'count': 1,
                         'slot': source, 'occupied': False,
                         'components': {'minecraft:equippable': None}})
    return rows


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
    assert digest(jar.read_bytes(), 'sha1') == sha1
    assert digest(mapping.read_bytes()) == mapping_hash
    cp = os.pathsep.join(str(Path(path).resolve()) for path in args.modern_classpath_file.read_text().strip().split(os.pathsep))
    classpath = original_modern_classpath(jar, cp)
    sources = ['scripts/' + name + '.java' for name in [
        'ExportInventoryTransfers', 'ExportItemComponents', 'ExportItemProperties',
        'ExportItemComponentSchema', 'ExportComponentValueRules', 'ExportNbtSemantics',
        'ExportComponentNormalization', 'ExportTextCore', 'ExportEnchantmentConstructors',
        'ExportEquipmentTransfers',
    ]]
    named = runtime / 'named-sources'
    named.mkdir(exist_ok=True)
    for path in sources:
        (named / Path(path).name).write_text('package voxrig.oracle;\n' + (ROOT / path).read_text())
    compiler = runtime / 'CompileOwnTool.java'
    compiler.write_text(COMPILER)
    inputs, raw = runtime / 'inputs.json', runtime / 'raw.json'
    request_bytes = encoded(requests(), compact=True)
    classes = runtime / 'own-classes'
    if not args.normalize_only:
        inputs.write_bytes(request_bytes)
        stages = [
            ('compile', [str(compiler), cp, str(classes), *(str(named / Path(path).name) for path in sources)]),
            ('run', ['-cp', str(classes) + os.pathsep + cp, 'voxrig.oracle.ExportEquipmentTransfers', str(inputs), str(raw)]),
        ]
        for label, command in stages:
            with (runtime / (label + '.log')).open('w') as log:
                subprocess.run(['java', '-Xmx512M', '-XX:ActiveProcessorCount=1', *command],
                               cwd=runtime, stdout=log, stderr=subprocess.STDOUT, check=True)
    assert inputs.read_bytes() == request_bytes
    data = json.loads(raw.read_text())
    rows = data['cases']
    assert len(rows) == len(requests())
    cases_path = 'data/client_api/equipment_transfer_cases-1.21.11.json.gz'
    cases = gzip.compress(encoded({'version': '1.21.11', 'cases': rows,
                                  'builtin_tags': data['builtin_tags']}, compact=True), mtime=0)
    source_path = 'data/client_api/equipment_transfer_source.json'
    source = encoded({'schema': 1,
                      'authority': 'Unchanged native modern equippable constructors, actual equipment slot acceptance and clicked QUICK_MOVE on original InventoryMenu.',
                      'scope': 'All eight equip slots, absent/empty/player-only/cow-only/named-tag allowed_entities, actual builtin tag declarations, added/replaced/removed equippable, partial armor capacity, occupied fallback, main/hotbar sources. Primitive context only, no live receive/mode/recovery proof.',
                      'original_server_jar_sha1': sha1, 'mappings_sha256': mapping_hash,
                      'original_classpath_entries_sha256': classpath,
                      'generators_sha256': {path: digest((ROOT / path).read_bytes()) for path in sources + ['scripts/export_equipment_transfers.py']},
                      'requests_sha256': digest(request_bytes), 'raw_output_sha256': digest(raw.read_bytes()),
                      'generated_compiler_sha256': digest(COMPILER.encode()), 'cases': len(rows),
                      'files_sha256': {cases_path: digest(cases)}})
    for name, content in [(cases_path, cases), (source_path, source)]:
        path = ROOT / name
        if args.check:
            assert path.read_bytes() == content, name
        else:
            path.write_bytes(content)
    print('native equipment transfers verified' if args.check else 'native equipment transfers generated', len(rows))


if __name__ == '__main__':
    main()
