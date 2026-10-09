#!/usr/bin/env python3
"""Observe original NBT/stream profile constructors without profile resolution."""
import argparse
import gzip
import json
import os
import struct
import subprocess
from pathlib import Path
from export_regular_clicks import ROOT, VERSIONS, digest, encoded, original_modern_classpath
from export_item_properties import COMPILER
from export_text_core import tag, scalar_json


def varint(value):
    out = bytearray()
    while value >= 128:
        out.append((value & 127) | 128); value >>= 7
    return bytes(out) + bytes([value])


def wire_string(value):
    value = value.encode('utf-8', errors='replace')
    return varint(len(value)) + value


def empty_lists(value):
    if isinstance(value, list):
        return [empty_lists(v) for v in value] if value else (9, b'\0\0\0\0\0')
    if isinstance(value, dict):
        return {k: empty_lists(v) for k, v in value.items()}
    return value


def requests():
    rows = []
    def nbt(name, value):
        kind, payload = tag(empty_lists(value))
        rows.append({'case': name, 'format': 'nbt', 'input_hex': (bytes([kind]) + payload).hex()})
    ident = (11, struct.pack('>iiiii', 4, 1, 2, 3, 4))
    for index, value in enumerate(['Voxrig', '', {}, {'name': 'Voxrig'}, {'id': ident}, {'name': 'Voxrig', 'id': ident}, {'properties': {}}, {'name': 'Voxrig', 'properties': {'textures': ['x']}}, {'id': ident, 'properties': {'textures': ['x']}}, {'name': 'Voxrig', 'id': ident, 'properties': {'textures': ['x']}}, 1, [], {'unknown': 'value'}]):
        nbt(f'base-{index}', value)
    for c in range(128):
        nbt(f'name-char-{c}', {'name': 'A' + chr(c) + 'B'})
    for length in [0, 1, 15, 16, 17]:
        nbt(f'name-length-{length}', 'a' * length)
    for name in ['日本語', '💎', '\ud800', ' leading', 'trailing ', 'line\nbreak']:
        nbt('name-unicode-' + str(len(rows)), {'name': name})
    for value in ['00000001-0000-0002-0000-000300000004', [1, 2, 3, 4], [1, 2, 3], [1, 2, 3, 4, 5], [1.9, -1.9, 0., 2147483648.], (7, struct.pack('>i', 4) + b'\1\2\3\4'), (12, struct.pack('>iqqqq', 4, 1, 2, 3, 4)), 1]:
        nbt('id-' + str(len(rows)), {'id': value})
    props = [{}, {'textures': []}, {'textures': ['a', 'b']}, {'textures': ['b', 'a']}, {'textures': ['a', 'a']}, {'a': ['x'], 'b': ['y']}, [{'name': 'a', 'value': 'x'}, {'name': 'b', 'value': 'y'}], [{'name': 'b', 'value': 'y'}, {'name': 'a', 'value': 'x'}], [{'name': 'a', 'value': 'x', 'signature': 's'}], [{'name': 'a', 'value': 'x', 'signature': ''}], [{'name': 'a', 'value': 'x', 'signature': 1}], {str(i): ['x'] for i in range(16)}, {str(i): ['x'] for i in range(17)}, {'textures': ['x'] * 17}, [{'name': 'a', 'value': 'x'}] * 16, [{'name': 'a', 'value': 'x'}] * 17, {'\ud800': ['x']}, {'textures': ['\ud800']}, [{'name': 'n' * 65, 'value': 'x'}], {'n' * 65: ['x']}, [{'name': 'a', 'value': 'v' * 32768}], {'a': ['v' * 32768]}, [{'name': 'a', 'value': 'x', 'signature': 's' * 1025}]]
    for index, properties in enumerate(props):
        nbt(f'properties-{index}', {'name': 'Voxrig', 'properties': properties})
    for field in ['texture', 'cape', 'elytra']:
        for value in ['foo', ':foo', 'minecraft:foo', 'textures/foo.png', '', 'INVALID', 1]:
            nbt(f'skin-{field}-{value}', {'name': 'Voxrig', field: value})
    for model in ['slim', 'wide', 'default', True, 1, 'SLIM', 'absent']:
        nbt('model-' + str(model), {'name': 'Voxrig', 'model': model})
    def stream(name, full, username, present_id, properties, skin=(None, None, None, None)):
        wire = bytes([int(full)])
        uid = struct.pack('>IIII', 1, 2, 3, 4)
        if full:
            wire += uid + wire_string(username)
        else:
            wire += b'\0' if username is None else b'\1' + wire_string(username)
            wire += b'\1' + uid if present_id else b'\0'
        wire += varint(len(properties))
        for key, value, signature in properties:
            wire += wire_string(key) + wire_string(value)
            wire += b'\0' if signature is None else b'\1' + wire_string(signature)
        for item in skin[:3]:
            wire += b'\0' if item is None else b'\1' + wire_string(item)
        wire += b'\0' if skin[3] is None else b'\1' + bytes([skin[3]])
        rows.append({'case': name, 'format': 'stream', 'input_hex': wire.hex()})
    for full in [False, True]:
        for username in [None, '', 'Voxrig', '日本語', ' leading']:
            if full and username is None:
                continue
            for present_id in [False, True]:
                for props_index, properties in enumerate([[], [('textures', 'x', None)], [('a', 'x', None), ('b', 'y', None)], [('b', 'y', None), ('a', 'x', None)], [('a', 'x', ''), ('a', 'x', 's')]]):
                    stream(f'stream-{full}-{username}-{present_id}-{props_index}', full, username, present_id, properties)
    for value in [None, 'foo', ':foo', 'INVALID']:
        for bit in [None, 0, 1, 2, 255]:
            stream(f'stream-skin-{value}-{bit}', False, 'Voxrig', False, [], (value, None, None, bit))
    for length in [64, 65]:
        stream(f'stream-property-name-{length}', False, None, False, [('n' * length, 'x', None)])
    stream('stream-signature-1025', False, 'Voxrig', False, [('a', 'x', 's' * 1025)])
    return rows


def main():
    p = argparse.ArgumentParser(description=__doc__)
    for name in ['downloads', 'modern-classpath-file', 'runtime-output']:
        p.add_argument('--' + name, type=Path, required=True)
    p.add_argument('--normalize-only', action='store_true'); p.add_argument('--check', action='store_true')
    a = p.parse_args(); b = a.runtime_output.resolve(); b.mkdir(parents=True, exist_ok=True)
    jar = (a.downloads / '1.21.11-server.jar').resolve(); mapping = a.downloads / '1.21.11-server-mappings.txt'
    sha1, msha = VERSIONS['1.21.11']
    if digest(jar.read_bytes(), 'sha1') != sha1 or digest(mapping.read_bytes()) != msha:
        raise SystemExit('original inputs differ')
    cp = os.pathsep.join(str(Path(s).resolve()) for s in a.modern_classpath_file.read_text().strip().split(os.pathsep)); classpath = original_modern_classpath(jar, cp)
    sources = ['scripts/ExportInventoryTransfers.java', 'scripts/ExportItemComponents.java', 'scripts/ExportItemProperties.java', 'scripts/ExportItemComponentSchema.java', 'scripts/ExportComponentValueRules.java', 'scripts/ExportNbtSemantics.java', 'scripts/ExportComponentNormalization.java', 'scripts/ExportProfiles.java']
    named = b / 'named-sources'; named.mkdir(exist_ok=True)
    for s in sources:
        (named / Path(s).name).write_text('package voxrig.oracle;\n' + (ROOT / s).read_text())
    compiler = b / 'CompileOwnTool.java'; compiler.write_text(COMPILER); classes = b / 'own-classes'; inputs = b / 'inputs.json'; raw = b / 'raw.json'; request_bytes = encoded(requests())
    if not a.normalize_only:
        inputs.write_bytes(request_bytes)
        for label, args in [('compile', [str(compiler), cp, str(classes), *(str(named / Path(s).name) for s in sources)]), ('run', ['-cp', str(classes) + os.pathsep + cp, 'voxrig.oracle.ExportProfiles', str(inputs), str(raw)])]:
            with (b / (label + '.log')).open('w') as log:
                subprocess.run(['java', '-Xmx512M', '-XX:ActiveProcessorCount=1', *args], cwd=b, stdout=log, stderr=subprocess.STDOUT, check=True)
    assert inputs.read_bytes() == request_bytes
    data = scalar_json(json.loads(raw.read_text()))
    rules_file = 'data/client_api/profile_rules-1.21.11.json'; rules_bytes = encoded(data['rules'])
    cases_file = 'data/client_api/profile_cases-1.21.11.json.gz'; case_bytes = gzip.compress(encoded({'schema': 1, 'cases': data['cases'], 'pairs': data['pairs']}, compact=True), mtime=0)
    source = encoded({'schema': 1, 'authority': 'Unchanged original NBT/stream profile constructors, constructor fields, native equals and original name/model rules. No network skin/profile resolution, persistent/cache or gameplay claim.', 'original_server_jar_sha1': sha1, 'mappings_sha256': msha, 'original_classpath_entries_sha256': classpath, 'java_version': data['java_version'], 'generators_sha256': {s: digest((ROOT / s).read_bytes()) for s in sources + ['scripts/export_profiles.py', 'scripts/export_text_core.py', 'scripts/export_item_properties.py', 'scripts/export_regular_clicks.py']}, 'generated_compiler_sha256': digest(COMPILER.encode()), 'requests_sha256': digest(request_bytes), 'raw_output_sha256': digest(raw.read_bytes()), 'local_original_profile_inspection_sha256': digest((b / 'original-profile-bytecode.log').read_bytes()), 'files_sha256': {rules_file: digest(rules_bytes), cases_file: digest(case_bytes)}, 'cases': len(data['cases']), 'accepted': sum(c['accepted'] for c in data['cases']), 'pairs': len(data['pairs'])})
    for name, content in [(rules_file, rules_bytes), (cases_file, case_bytes), ('data/client_api/profile_source.json', source)]:
        path = ROOT / name
        if a.check:
            if path.read_bytes() != content:
                raise SystemExit('native profile facts differ: ' + name)
        else:
            path.write_bytes(content)
    print('Original profile facts verified' if a.check else 'Original profile facts generated', len(data['cases']), sum(c['accepted'] for c in data['cases']), len(data['pairs']))


if __name__ == '__main__':
    main()
