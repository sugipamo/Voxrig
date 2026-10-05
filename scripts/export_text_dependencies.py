#!/usr/bin/env python3
"""Observe unchanged modern text item/dialog constructors, getters and equality."""
import argparse
import gzip
import json
import os
import subprocess
import struct
import shutil
from pathlib import Path
from export_regular_clicks import ROOT, VERSIONS, digest, encoded, original_modern_classpath
from export_item_properties import COMPILER
from export_text_core import tag, scalar_json


def requests():
    base = ROOT / 'data/client_api/text_constructor_cases-1.21.11.json.gz'
    rows = [{'case': r['case'], 'input_hex': r['input_hex']} for r in json.loads(gzip.decompress(base.read_bytes()))['cases']]
    probes = []
    for item in ['minecraft:stone', 'stone', 'minecraft:air', 'minecraft:diamond_sword', 'example:missing', ':stone', 'Minecraft:stone', '']:
        probes.append(('item-id-' + item, {'hover_event': {'action': 'show_item', 'id': item}}))
    for kind, patterns in [(1, [b'\x00', b'\xff', b'\x01', b'\x7f']), (2, [struct.pack('>h', v) for v in [-1, 0, 1, 64, 128]]), (3, [struct.pack('>i', v) for v in [-2147483648, -1, 0, 1, 2, 64, 99, 128, 2147483647]]), (4, [struct.pack('>q', v) for v in [-1, 0, 1, 2147483648, 9223372036854775807]]), (5, [struct.pack('>f', v) for v in [-1.9, 0., .9, 1.9, 2.9, float('nan'), float('inf')]]), (6, [struct.pack('>d', v) for v in [-1.9, .9, 1.9, float('nan'), float('inf')]])]:
        for value in patterns:
            probes.append((f'item-count-{kind}-{value.hex()}', {'hover_event': {'action': 'show_item', 'id': 'minecraft:stone', 'count': (kind, value)}}))
    for count in ['2', {}, [1]]:
        probes.append(('item-count-type-' + str(count), {'hover_event': {'action': 'show_item', 'id': 'minecraft:stone', 'count': count}}))
    for item in [None, 1, {}, ['stone']]:
        event = {'action': 'show_item'}
        if item is not None: event['id'] = item
        probes.append(('item-id-type-' + str(item), {'hover_event': event}))
    for patch in [{}, {'minecraft:max_stack_size': 64}, {'minecraft:max_stack_size': 1}, {'minecraft:max_stack_size': 0}, {'minecraft:damage': 3}, {'!minecraft:max_stack_size': {}}, {'minecraft:missing': 1}, 1, 'bad']:
        probes.append(('item-components-' + str(patch), {'hover_event': {'action': 'show_item', 'id': 'minecraft:stone', 'components': patch}}))
    for value in ['minecraft:custom_options', 'custom_options', 'minecraft:quick_actions', 'minecraft:server_links', 'example:missing', ':bad', 'Bad:bad', '', 1, {}, {'type': 'minecraft:notice', 'title': 'Voxrig'}, {'type': 'notice', 'title': {'text': 'Voxrig'}}, {'type': 'notice'}, {'title': 'Voxrig'}]:
        probes.append(('dialog-' + str(value), {'click_event': {'action': 'show_dialog', 'dialog': value}}))
    for event in [{'action': 'show_item', 'id': 'minecraft:stone', 'count': 2}, {'action': 'show_item', 'id': 'example:missing'}]:
        child = {'text': 'child', 'hover_event': event}
        for name, fields in [('nested-sibling', {'text': 'root', 'extra': [child]}), ('nested-argument', {'translate': 'voxrig.message', 'with': [child]}), ('nested-selector', {'selector': '@a', 'separator': child}), ('nested-nbt', {'nbt': 'path', 'storage': 'minecraft:foo', 'separator': child}), ('nested-hover-text', {'text': 'root', 'hover_event': {'action': 'show_text', 'value': child}}), ('nested-entity-name', {'text': 'root', 'hover_event': {'action': 'show_entity', 'id': 'minecraft:pig', 'uuid': '00000000-0000-0000-0000-000000000001', 'name': child}})]:
            kind, value = tag(fields)
            rows.append({'case': name + '-' + event['id'], 'input_hex': (bytes([kind]) + value).hex()})
    unimplemented = {'text': 'child', 'hover_event': {'action': 'show_item', 'id': 'minecraft:stone', 'components': {'minecraft:unbreakable': {}}}}
    for name, fields in [('unresolved-separator', {'nbt': 'path', 'storage': 'minecraft:foo', 'separator': unimplemented}), ('unresolved-fuzzy', {'translate': 'voxrig.message', 'with': [unimplemented], 'keybind': 'key.jump'}), ('invalid-dialog-separator', {'nbt': 'path', 'storage': 'minecraft:foo', 'separator': {'text': 'child', 'click_event': {'action': 'show_dialog', 'dialog': 1}}})]:
        kind, value = tag(fields)
        rows.append({'case': name, 'input_hex': (bytes([kind]) + value).hex()})
    for name, fields in probes:
        kind, value = tag({'text': 'Voxrig', **fields})
        rows.append({'case': name, 'input_hex': (bytes([kind]) + value).hex()})
    return rows


def main():
    p = argparse.ArgumentParser(description=__doc__)
    for name in ['downloads', 'modern-classpath-file', 'runtime-output']:
        p.add_argument('--' + name, type=Path, required=True)
    p.add_argument('--normalize-only', action='store_true')
    p.add_argument('--check', action='store_true')
    a = p.parse_args(); b = a.runtime_output.resolve(); b.mkdir(parents=True, exist_ok=True)
    jar = (a.downloads / '1.21.11-server.jar').resolve(); mapping = a.downloads / '1.21.11-server-mappings.txt'
    sha1, msha = VERSIONS['1.21.11']
    if digest(jar.read_bytes(), 'sha1') != sha1 or digest(mapping.read_bytes()) != msha:
        raise SystemExit('original inputs differ')
    cp = os.pathsep.join(str(Path(s).resolve()) for s in a.modern_classpath_file.read_text().strip().split(os.pathsep))
    classpath = original_modern_classpath(jar, cp)
    sources = ['scripts/ExportInventoryTransfers.java', 'scripts/ExportItemComponents.java', 'scripts/ExportItemProperties.java', 'scripts/ExportItemComponentSchema.java', 'scripts/ExportComponentValueRules.java', 'scripts/ExportNbtSemantics.java', 'scripts/ExportComponentNormalization.java', 'scripts/ExportTextCore.java', 'scripts/ExportTextDependencies.java']
    named = b / 'named-sources'; named.mkdir(exist_ok=True)
    for s in sources:
        (named / Path(s).name).write_text('package voxrig.oracle;\n' + (ROOT / s).read_text())
    compiler = b / 'CompileOwnTool.java'; compiler.write_text(COMPILER)
    classes = b / 'own-classes'; inputs = b / 'inputs.json'; raw = b / 'raw.json'; request_bytes = encoded(requests())
    if not a.normalize_only:
        inputs.write_bytes(request_bytes)
        for label, args in [('compile', [str(compiler), cp, str(classes), *(str(named / Path(s).name) for s in sources)]), ('run', ['-cp', str(classes) + os.pathsep + cp, 'voxrig.oracle.ExportTextDependencies', str(inputs), str(raw)])]:
            with (b / (label + '.log')).open('w') as log:
                subprocess.run(['java', '-Xmx512M', '-XX:ActiveProcessorCount=1', *args], cwd=b, stdout=log, stderr=subprocess.STDOUT, check=True)
    assert inputs.read_bytes() == request_bytes
    data = scalar_json(json.loads(raw.read_text()))
    case_file = 'data/client_api/text_dependency_cases-1.21.11.json.gz'; case_bytes = gzip.compress(encoded({'schema': 1, 'cases': data['cases'], 'pairs': data['pairs']}, compact=True), mtime=0)
    java = Path(shutil.which('java')).resolve()
    source = encoded({'schema': 1, 'authority': 'Unchanged original text dependency decoding, original hover ItemStack getter and stream encoding, dialog holder getter and whole component.equals. Includes prior419 constructors and focused item/dialog probes. Not live server/cache/action evidence.', 'original_server_jar_sha1': sha1, 'mappings_sha256': msha, 'java_version': data['java_version'], 'java_executable_sha256': digest(java.read_bytes()), 'jdk_modules_sha256': digest((java.parent.parent / 'lib/modules').read_bytes()), 'original_classpath_entries_sha256': classpath, 'generators_sha256': {s: digest((ROOT / s).read_bytes()) for s in sources + ['scripts/export_text_dependencies.py', 'scripts/export_text_core.py', 'scripts/export_item_properties.py', 'scripts/export_regular_clicks.py', 'data/client_api/text_constructor_cases-1.21.11.json.gz']}, 'generated_compiler_sha256': digest(COMPILER.encode()), 'requests_sha256': digest(request_bytes), 'raw_output_sha256': digest(raw.read_bytes()), 'files_sha256': {case_file: digest(case_bytes)}, 'cases': len(data['cases']), 'accepted': sum(c['accepted'] for c in data['cases']), 'pairs': len(data['pairs'])})
    for name, content in [(case_file, case_bytes), ('data/client_api/text_dependency_source.json', source)]:
        path = ROOT / name
        if a.check:
            if path.read_bytes() != content:
                raise SystemExit('native constructor facts differ: ' + name)
        else:
            path.write_bytes(content)
    print('Original constructor facts verified' if a.check else 'Original constructor facts generated', len(data['cases']), sum(c['accepted'] for c in data['cases']), len(data['pairs']))


if __name__ == '__main__':
    main()
