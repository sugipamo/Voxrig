#!/usr/bin/env python3
"""Pin exact equipment PICKUP predictions to unchanged official menu primitives."""
import argparse
import gzip
import os
from pathlib import Path
import subprocess
import zipfile

from export_item_properties import COMPILER
from export_regular_clicks import ROOT, VERSIONS, digest, encoded, original_modern_classpath


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--downloads', type=Path, required=True)
    parser.add_argument('--runtime-output', type=Path, required=True)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    runtime = args.runtime_output.resolve()
    runtime.mkdir(parents=True, exist_ok=True)
    jars = {v: (args.downloads / (v + '-server.jar')).resolve() for v in VERSIONS}
    for v, jar in jars.items():
        assert digest(jar.read_bytes(), 'sha1') == VERSIONS[v][0]
    entries = []
    with zipfile.ZipFile(jars['1.21.11']) as bundle:
        for group in ('versions', 'libraries'):
            for line in bundle.read('META-INF/' + group + '.list').decode().splitlines():
                sha, _, name = line.split('\t')
                content = bundle.read('META-INF/' + group + '/' + name)
                assert digest(content) == sha
                target = runtime / 'original-bundle' / group / name
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_bytes(content)
                entries.append(str(target))
    cp = os.pathsep.join(entries)
    original_modern_classpath(jars['1.21.11'], cp)
    sources = ['scripts/ExportInventoryTransfers.java', 'scripts/ExportEquipmentPickups.java']
    named = runtime / 'named-sources'
    named.mkdir(exist_ok=True)
    for name in sources:
        (named / Path(name).name).write_text('package voxrig.oracle;\n' + (ROOT / name).read_text())
    compiler = runtime / 'CompileOwnTool.java'
    compiler.write_text(COMPILER)
    classes = runtime / 'own-classes'
    with (runtime / 'compile.log').open('w') as log:
        subprocess.run(['java', '-Xmx512M', '-XX:ActiveProcessorCount=1', str(compiler), cp,
                        str(classes), *(str(named / Path(s).name) for s in sources)],
                       cwd=runtime, stdout=log, stderr=subprocess.STDOUT, check=True)
    outputs, runs = {}, []
    for v, jar in jars.items():
        raw = runtime / (v + '-raw.json')
        with (runtime / (v + '-run.log')).open('w') as log:
            subprocess.run(['java', '-Xmx512M', '-XX:ActiveProcessorCount=1', '-cp',
                            str(classes) + os.pathsep + (str(jar) if v == '1.16.1' else cp),
                            'voxrig.oracle.ExportEquipmentPickups', v, str(raw)],
                           cwd=runtime, stdout=log, stderr=subprocess.STDOUT, check=True)
        import json
        facts = json.loads(raw.read_text())
        assert facts['version'] == v and len(facts['cases']) == 7290
        path = 'data/client_api/equipment_pickup_cases-' + v + '.json.gz'
        outputs[path] = gzip.compress(encoded(facts, compact=True), mtime=0)
        runs.append(dict(version=v, original_server_jar_sha1=VERSIONS[v][0],
                         cases=len(facts['cases']), raw_output_sha256=digest(raw.read_bytes()),
                         files_sha256={path: digest(outputs[path])}))
    outputs['data/client_api/equipment_pickup_source.json'] = encoded(dict(
        schema=1, authority='Unchanged official InventoryMenu clicked PICKUP on slots 5..8 and 45.',
        limits='Default stack primitive predictions only; not live receipt, cancellation, curse/component or mode/world qualification.',
        generators_sha256={p: digest((ROOT / p).read_bytes()) for p in sources + ['scripts/export_equipment_pickups.py']},
        generated_compiler_sha256=digest(COMPILER.encode()), runs=runs))
    for path, content in outputs.items():
        if args.check:
            assert (ROOT / path).read_bytes() == content, path
        else:
            (ROOT / path).write_bytes(content)
    print('Exact equipment native PICKUP evidence verified' if args.check else 'Exact equipment native PICKUP evidence generated')


if __name__ == '__main__':
    main()
