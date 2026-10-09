#!/usr/bin/env python3
"""Record original modern fuzzy constructor order, getters and rejection facts."""
import argparse
import gzip
import json
import os
import subprocess
from pathlib import Path
from export_regular_clicks import ROOT, VERSIONS, digest, encoded, original_modern_classpath
from export_item_properties import COMPILER
from export_text_core import tag, scalar_json


def requests():
    baseline = json.loads(gzip.decompress((ROOT / 'data/client_api/text_core_cases-1.21.11.json.gz').read_bytes()))
    rows = [{'case': c['case'], 'input_hex': c['input_hex']} for c in baseline['cases']]
    kinds = {
        'text': {'text': 'Voxrig'}, 'translatable': {'translate': 'voxrig.message'},
        'keybind': {'keybind': 'key.jump'}, 'score': {'score': {'name': '*', 'objective': 'voxrig'}},
        'selector': {'selector': '@a'}, 'nbt': {'nbt': 'value', 'storage': 'data'},
        'atlas': {'sprite': 'block/stone'}, 'player': {'player': 'Voxrig'},
    }
    probes = []
    for a, first in kinds.items():
        for b, second in kinds.items():
            if a == b:
                continue
            probes.append((f'fuzzy-valid-{a}-{b}', {**first, **second}))
            probes.append((f'fuzzy-invalid-{a}-{b}', {**{k: 1 for k in first}, **second}))
            probes.append((f'fuzzy-reversed-{a}-{b}', {**second, **first}))
    for name, fields in kinds.items():
        kind = 'object' if name in ('atlas', 'player') else name
        for explicit in [kind, 'absent', 1]:
            probes.append((f'explicit-{name}-{explicit}', {**fields, 'type': explicit}))
    for source in ['block', 'entity', 'storage']:
        value = {'block': '~ ~ ~', 'entity': '@a', 'storage': 'data'}[source]
        for other in ['block', 'entity', 'storage']:
            if source == other:
                continue
            other_value = {'block': '~ ~ ~', 'entity': '@a', 'storage': 'data'}[other]
            for invalid in [False, True]:
                probes.append((f'source-{source}-{other}-{invalid}', {'nbt': 'value', source: 1 if invalid else value, other: other_value}))
        for explicit in [source, 'absent', 1]:
            probes.append((f'source-explicit-{source}-{explicit}', {'nbt': 'value', source: value, 'source': explicit}))
    for value in [1, {}, [], 'Voxrig']:
        probes.append((f'translate-fallback-{value}', {'translate': 'voxrig.message', 'fallback': value}))
    for field in ['with', 'hat', 'atlas', 'object']:
        for value in [1, 'absent', {}]:
            fields = {'translate': 'voxrig.message'} if field == 'with' else {'player': 'Voxrig', 'sprite': 'block/stone'}
            probes.append((f'optional-invalid-{field}-{value}', {**fields, field: value}))
    for value in [1, 0, 'absent', {}]:
        probes.append((f'player-hat-{value}', {'player': 'Voxrig', 'hat': value}))
    for object_kind in ['atlas', 'player', 'absent', 1]:
        probes.append((f'object-explicit-{object_kind}', {'sprite': 'block/stone', 'player': 'Voxrig', 'object': object_kind}))
    for name, fields in probes:
        # Empty list is an explicit TAG_List of End, not an invented value.
        if fields.get('fallback') == []:
            fields = {**fields, 'fallback': (9, b'\0\0\0\0\0')}
        kind, value = tag(fields)
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
    sources = ['scripts/ExportInventoryTransfers.java', 'scripts/ExportItemComponents.java', 'scripts/ExportItemProperties.java', 'scripts/ExportItemComponentSchema.java', 'scripts/ExportComponentValueRules.java', 'scripts/ExportNbtSemantics.java', 'scripts/ExportComponentNormalization.java', 'scripts/ExportTextCore.java', 'scripts/ExportTextConstructors.java']
    named = b / 'named-sources'; named.mkdir(exist_ok=True)
    for s in sources:
        (named / Path(s).name).write_text('package voxrig.oracle;\n' + (ROOT / s).read_text())
    compiler = b / 'CompileOwnTool.java'; compiler.write_text(COMPILER)
    classes = b / 'own-classes'; inputs = b / 'inputs.json'; raw = b / 'raw.json'; request_bytes = encoded(requests())
    if not a.normalize_only:
        inputs.write_bytes(request_bytes)
        for label, args in [('compile', [str(compiler), cp, str(classes), *(str(named / Path(s).name) for s in sources)]), ('run', ['-cp', str(classes) + os.pathsep + cp, 'voxrig.oracle.ExportTextConstructors', str(inputs), str(raw)])]:
            with (b / (label + '.log')).open('w') as log:
                subprocess.run(['java', '-Xmx512M', '-XX:ActiveProcessorCount=1', *args], cwd=b, stdout=log, stderr=subprocess.STDOUT, check=True)
    assert inputs.read_bytes() == request_bytes
    data = scalar_json(json.loads(raw.read_text()))
    rules_file = 'data/client_api/text_constructor_rules-1.21.11.json'; rule_bytes = encoded(data['rules'])
    case_file = 'data/client_api/text_constructor_cases-1.21.11.json.gz'; case_bytes = gzip.compress(encoded({'schema': 1, 'cases': data['cases'], 'pairs': data['pairs']}, compact=True), mtime=0)
    source = encoded({'schema': 1, 'authority': 'Unchanged original text stream decoding and getter fields, original native fuzzy mapper order and component.equals. Complex selector/profile/URI/dialog/item/entity validation remains pending; not persistent/cache/gameplay evidence.', 'original_server_jar_sha1': sha1, 'mappings_sha256': msha, 'original_classpath_entries_sha256': classpath, 'generators_sha256': {s: digest((ROOT / s).read_bytes()) for s in sources + ['scripts/export_text_constructors.py', 'scripts/export_text_core.py', 'scripts/export_item_properties.py', 'scripts/export_regular_clicks.py', 'data/client_api/text_core_cases-1.21.11.json.gz']}, 'generated_compiler_sha256': digest(COMPILER.encode()), 'requests_sha256': digest(request_bytes), 'raw_output_sha256': digest(raw.read_bytes()), 'local_original_constructor_inspection_sha256': digest((b / 'original-constructor-bytecode.log').read_bytes()), 'files_sha256': {rules_file: digest(rule_bytes), case_file: digest(case_bytes)}, 'cases': len(data['cases']), 'accepted': sum(c['accepted'] for c in data['cases']), 'pairs': len(data['pairs'])})
    for name, content in [(rules_file, rule_bytes), (case_file, case_bytes), ('data/client_api/text_constructor_source.json', source)]:
        path = ROOT / name
        if a.check:
            if path.read_bytes() != content:
                raise SystemExit('native constructor facts differ: ' + name)
        else:
            path.write_bytes(content)
    print('Original constructor facts verified' if a.check else 'Original constructor facts generated', len(data['cases']), sum(c['accepted'] for c in data['cases']), len(data['pairs']))


if __name__ == '__main__':
    main()
