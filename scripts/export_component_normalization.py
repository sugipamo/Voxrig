#!/usr/bin/env python3
"""Native Identifier stream normalization and adverse text constructor/equality facts.

Text facts are an independent oracle for future semantics, not a text implementation.
"""
import argparse
import gzip
import json
import os
import subprocess
from pathlib import Path
from export_regular_clicks import ROOT, VERSIONS, digest, encoded, original_modern_classpath
from export_item_properties import COMPILER
from export_item_component_schema import normalize


def requests():
    cases = []

    def add(name, value):
        cases.append({'case': name, 'value': value})

    for name, value in [('literal-empty', ''), ('record-empty', {'text': ''}),
                        ('literal', 'Voxrig'), ('record', {'text': 'Voxrig'}),
                        ('singleton-list', ['Voxrig']), ('empty-list', []),
                        ('list', ['Voxrig', 'tail']),
                        ('record-extra', {'text': 'Voxrig', 'extra': ['tail']}),
                        ('empty-extra', {'text': 'Voxrig', 'extra': []}),
                        ('empty-parent', {'text': '', 'extra': ['Voxrig']}),
                        ('unknown-key', {'text': 'Voxrig', 'unknown': 'ignored'}),
                        ('empty-map', {}), ('number-root', 1), ('boolean-root', True),
                        ('null-root', None), ('text-number', {'text': 1}),
                        ('text-null', {'text': None}), ('type-literal', {'type': 'text', 'text': 'Voxrig'}),
                        ('wrong-type', {'type': 'absent', 'text': 'Voxrig'})]:
        add(name, value)
    for field in ['bold', 'italic', 'underlined', 'strikethrough', 'obfuscated']:
        for name, value in [('true', True), ('false', False), ('null', None), ('integer', 1), ('string', 'true')]:
            add(field + '-' + name, {'text': 'Voxrig', field: value})
    for color in ['red', '#ff5555', '#FF5555', '#00ff5555', '#+ff5555', '#-0',
                  'black', '#0', '#000000', 'white', '#ffffff', '#1000000',
                  '#-1', 'RED', 'reset', 'absent', None, 0]:
        add('color-' + str(color), {'text': 'Voxrig', 'color': color})
    for value in [0, -1, 2147483647, -2147483648, [1., .5, 0., 1.], [1., 1., 1.], None, 'red']:
        add('shadow-' + str(value), {'text': 'Voxrig', 'shadow_color': value})
    for name, value in [('empty', ''), ('text', 'insert'), ('null', None), ('number', 1)]:
        add('insertion-' + name, {'text': 'Voxrig', 'insertion': value})
    for value in ['default', ':default', 'minecraft:default', 'voxrig:font', 'INVALID',
                  {'type': 'resource', 'id': 'default'},
                  {'type': 'sprite', 'atlas': 'blocks', 'sprite': 'block/stone'},
                  {'type': 'player', 'player': {'name': 'Voxrig'}}, None]:
        add('font-' + str(value), {'text': 'Voxrig', 'font': value})
    for name, value in [
        ('translate', {'translate': 'voxrig.message'}),
        ('translate-fallback', {'translate': 'voxrig.message', 'fallback': 'fallback'}),
        ('translate-empty-with', {'translate': 'voxrig.message', 'with': []}),
        ('translate-args', {'translate': 'voxrig.message', 'with': ['arg', {'text': 'nested', 'italic': True}, 1, True]}),
        ('translate-one', {'translate': 'voxrig.message', 'with': [1]}),
        ('translate-one-float', {'translate': 'voxrig.message', 'with': [1.]}),
        ('translate-large', {'translate': 'voxrig.message', 'with': [9007199254740993]}),
        ('translate-null', {'translate': 'voxrig.message', 'with': [None]}),
        ('keybind', {'keybind': 'key.jump'}),
        ('score', {'score': {'name': '@s', 'objective': 'voxrig'}}),
        ('score-name', {'score': {'name': 'Voxrig', 'objective': 'voxrig'}}),
        ('selector', {'selector': '@a'}),
        ('selector-separator', {'selector': '@a', 'separator': {'text': '|'}}),
        ('selector-empty-separator', {'selector': '@a', 'separator': ''}),
        ('block-nbt', {'nbt': 'Items[0].id', 'block': '~ ~ ~'}),
        ('entity-nbt', {'nbt': 'CustomName', 'entity': '@s'}),
        ('storage-nbt', {'nbt': 'value', 'storage': 'voxrig:data'}),
        ('storage-nbt-alias', {'nbt': 'value', 'storage': 'data'}),
        ('storage-nbt-false', {'nbt': 'value', 'storage': 'data', 'interpret': False}),
        ('storage-nbt-true', {'nbt': 'value', 'storage': 'data', 'interpret': True, 'separator': '|'}),
        ('sprite', {'atlas': 'minecraft:blocks', 'sprite': 'minecraft:block/stone'}),
        ('sprite-typed', {'type': 'object', 'atlas': 'minecraft:blocks', 'sprite': 'minecraft:block/stone'}),
        ('player-sprite', {'player': {'name': 'Voxrig'}}),
        ('player-sprite-no-hat', {'player': {'name': 'Voxrig'}, 'hat': False}),
    ]:
        add(name, value)
    for name, event in [
        ('open-url', {'action': 'open_url', 'url': 'https://example.org'}),
        ('run', {'action': 'run_command', 'command': '/say voxrig'}),
        ('suggest', {'action': 'suggest_command', 'command': '/say voxrig'}),
        ('page', {'action': 'change_page', 'page': 2}),
        ('copy', {'action': 'copy_to_clipboard', 'value': 'Voxrig'}),
        ('file', {'action': 'open_file', 'path': '/tmp/voxrig'}),
        ('custom', {'action': 'custom', 'id': 'voxrig:event', 'payload': {'value': 1}}),
        ('dialog', {'action': 'show_dialog', 'dialog': 'minecraft:custom_options'}),
        ('unknown', {'action': 'absent'}),
        ('old-value', {'action': 'run_command', 'value': '/say voxrig'}),
    ]:
        add('click-' + name, {'text': 'Voxrig', 'click_event': event})
    for name, event in [
        ('text', {'action': 'show_text', 'value': 'hover'}),
        ('record-text', {'action': 'show_text', 'value': {'text': 'hover'}}),
        ('styled-text', {'action': 'show_text', 'value': {'text': 'hover', 'color': '#ff5555'}}),
        ('item', {'action': 'show_item', 'id': 'minecraft:stone'}),
        ('item-count', {'action': 'show_item', 'id': 'minecraft:stone', 'count': 2}),
        ('entity', {'action': 'show_entity', 'id': [0, 0, 0, 1], 'type': 'minecraft:pig', 'name': 'Pig'}),
        ('unknown', {'action': 'absent'}),
    ]:
        add('hover-' + name, {'text': 'Voxrig', 'hover_event': event})
    add('combined', {'text': 'Voxrig', 'color': 'red', 'bold': True, 'italic': False,
                     'shadow_color': 0, 'insertion': 'insert', 'font': 'default',
                     'extra': [{'translate': 'voxrig.message', 'with': ['argument']}],
                     'click_event': {'action': 'copy_to_clipboard', 'value': 'copy'},
                     'hover_event': {'action': 'show_text', 'value': 'hover'}})
    return cases


def identifier_case(row):
    """Retain Java UTF-16 when the diagnostic is not a Unicode scalar string."""
    row = dict(row)
    for name in ['input', 'failure']:
        if name not in row:
            continue
        value = row[name]
        units = value.encode('utf-16-be', errors='surrogatepass')
        if name == 'input':
            row['input_utf16'] = [int.from_bytes(units[i:i + 2], 'big') for i in range(0, len(units), 2)]
        try:
            value.encode('utf-8')
        except UnicodeEncodeError:
            row[name + '_utf16'] = [int.from_bytes(units[i:i + 2], 'big') for i in range(0, len(units), 2)]
            row[name] = None
    return row


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--downloads', type=Path, required=True)
    p.add_argument('--modern-classpath-file', type=Path, required=True)
    p.add_argument('--runtime-output', type=Path, required=True)
    p.add_argument('--normalize-only', action='store_true')
    p.add_argument('--check', action='store_true')
    a = p.parse_args()
    b = a.runtime_output.resolve()
    b.mkdir(parents=True, exist_ok=True)
    jar = (a.downloads / '1.21.11-server.jar').resolve()
    mapping = a.downloads / '1.21.11-server-mappings.txt'
    sha1, msha = VERSIONS['1.21.11']
    if digest(jar.read_bytes(), 'sha1') != sha1 or digest(mapping.read_bytes()) != msha:
        raise SystemExit('original inputs differ')
    cp = os.pathsep.join(str(Path(s).resolve()) for s in a.modern_classpath_file.read_text().strip().split(os.pathsep))
    classpath = original_modern_classpath(jar, cp)
    sources = ['scripts/ExportInventoryTransfers.java', 'scripts/ExportItemComponents.java',
               'scripts/ExportItemProperties.java', 'scripts/ExportItemComponentSchema.java',
               'scripts/ExportComponentValueRules.java', 'scripts/ExportNbtSemantics.java',
               'scripts/ExportComponentNormalization.java', 'scripts/ExportResourceIdentifiers.java']
    named = b / 'named-sources'
    named.mkdir(exist_ok=True)
    for source in sources:
        (named / Path(source).name).write_text('package voxrig.oracle;\n' + (ROOT / source).read_text())
    compiler = b / 'CompileOwnTool.java'
    compiler.write_text(COMPILER)
    classes, raw, inputs = b / 'own-classes', b / 'raw.json', b / 'inputs.json'
    legacy_jar = (a.downloads / '1.16.1-server.jar').resolve()
    legacy_mapping = a.downloads / '1.16.1-server-mappings.txt'
    legacy_sha1, legacy_msha = VERSIONS['1.16.1']
    if digest(legacy_jar.read_bytes(), 'sha1') != legacy_sha1 or digest(legacy_mapping.read_bytes()) != legacy_msha:
        raise SystemExit('original legacy inputs differ')
    legacy_inputs, legacy_raw = b / 'legacy-inputs.json', b / 'legacy-raw.json'
    request_bytes = encoded(requests())
    if not a.normalize_only:
        inputs.write_bytes(request_bytes)
        for label, args in [
            ('compile', [str(compiler), cp, str(classes), *(str(named / Path(s).name) for s in sources)]),
            ('run', ['-cp', str(classes) + os.pathsep + cp, 'voxrig.oracle.ExportComponentNormalization', str(inputs), str(raw)])
        ]:
            with (b / (label + '.log')).open('w') as log:
                subprocess.run(['java', '-Xmx512M', '-XX:ActiveProcessorCount=1', *args], cwd=b,
                               stdout=log, stderr=subprocess.STDOUT, check=True)
    assert inputs.read_bytes() == request_bytes, 'saved native requests differ'
    data = json.loads(raw.read_text())
    legacy_request_bytes = encoded([v['input'] for v in data['identifiers']])
    if not a.normalize_only:
        legacy_inputs.write_bytes(legacy_request_bytes)
        with (b / 'legacy-run.log').open('w') as log:
            subprocess.run(['java', '-Xmx512M', '-XX:ActiveProcessorCount=1', '-cp',
                            str(classes) + os.pathsep + str(legacy_jar),
                            'voxrig.oracle.ExportResourceIdentifiers', str(legacy_inputs), str(legacy_raw)],
                           cwd=b, stdout=log, stderr=subprocess.STDOUT, check=True)
    assert legacy_inputs.read_bytes() == legacy_request_bytes, 'saved legacy requests differ'
    data['legacy_identifiers'] = json.loads(legacy_raw.read_text())
    assert len(data['legacy_identifiers']) == len(data['identifiers'])
    for legacy, modern in zip(data['legacy_identifiers'], data['identifiers']):
        assert legacy['input'] == modern['input'] and legacy['accepted'] == modern['accepted']
        if legacy['accepted']:
            assert (legacy['namespace'], legacy['path']) == (modern['namespace'], modern['path'])
    data['identifiers'] = [identifier_case(row) for row in data['identifiers']]
    data['legacy_identifiers'] = [identifier_case(row) for row in data['legacy_identifiers']]
    schema = json.loads((ROOT / 'data/client_api/item_component_schema-1.21.11.json').read_text())
    assert normalize(data['graph']) == schema, 'loaded native composition differs'
    ids = [v for v in data['forward'] if v.get('normalization') == 'identifier']
    assert len(ids) == 1 and ids[0]['codec_class'] == 'aao$14'
    node = ids[0]['node']
    assert schema['nodes'][node]['op'] == 'forward'
    assert schema['nodes'][schema['nodes'][node]['child']]['op'] == 'string'
    assert {v['case'] for v in data['text']} == {v['case'] for v in requests()}
    assert all(v['independent_decode_equal'] for v in data['text'] if v['accepted'])
    rules_file = 'data/client_api/component_normalization_rules-1.21.11.json'
    rules = encoded({'schema': 1, 'identifiers': [{'node': node, 'child': schema['nodes'][node]['child'],
                                                'codec_class': ids[0]['codec_class'], 'value_class': 'amo'}]})
    cases_file = 'data/client_api/component_normalization_cases-1.21.11.json.gz'
    corpus = gzip.compress(encoded({'schema': 1, **{k: data[k] for k in ['forward', 'identifiers', 'legacy_identifiers', 'text', 'text_pairs']}}, compact=True), mtime=0)
    source = encoded({
        'schema': 1,
        'authority': 'Unchanged original Identifier stream constructor, arbitrary-tag text stream decode, persistent text encoder, component.equals and original ItemStack data/matches comparisons. Text/forward facts remain an oracle, not a complete runtime component/equality/hash implementation or gameplay proof.',
        'original_server_jar_sha1': sha1, 'mappings_sha256': msha,
        'legacy_original_server_jar_sha1': legacy_sha1, 'legacy_mappings_sha256': legacy_msha,
        'original_classpath_entries_sha256': classpath,
        'generators_sha256': {s: digest((ROOT / s).read_bytes()) for s in sources + [
            'scripts/export_component_normalization.py', 'scripts/export_item_component_schema.py',
            'scripts/export_item_properties.py', 'scripts/export_item_components.py',
            'scripts/export_regular_clicks.py', 'data/client_api/item_component_schema-1.21.11.json']},
        'generated_compiler_sha256': digest(COMPILER.encode()), 'requests_sha256': digest(request_bytes),
        'raw_output_sha256': digest(raw.read_bytes()),
        'legacy_requests_sha256': digest(legacy_request_bytes), 'legacy_raw_output_sha256': digest(legacy_raw.read_bytes()),
        'local_original_normalization_inspection_sha256': digest((b / 'original-normalization-bytecode.log').read_bytes()),
        'local_original_legacy_identifier_inspection_sha256': digest((b / 'original-legacy-identifiers-bytecode.log').read_bytes()),
        'files_sha256': {rules_file: digest(rules), cases_file: digest(corpus)},
        'forward_nodes': len(data['forward']), 'identifier_cases': len(data['identifiers']),
        'text_cases': len(data['text']), 'accepted_text_cases': sum(v['accepted'] for v in data['text']),
        'text_equality_pairs': len(data['text_pairs']),
    })
    for name, content in [(rules_file, rules), (cases_file, corpus),
                          ('data/client_api/component_normalization_source.json', source)]:
        path = ROOT / name
        if a.check:
            if path.read_bytes() != content:
                raise SystemExit('original component normalization facts differ: ' + name)
        else:
            path.write_bytes(content)
    print('Original component normalization facts verified' if a.check else 'Original component normalization facts generated',
          len(data['forward']), len(data['identifiers']), len(data['text']), len(data['text_pairs']))


if __name__ == '__main__':
    main()
