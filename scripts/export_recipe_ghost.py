#!/usr/bin/env python3
"""Observe original recipe ghost response codecs, serially."""
import argparse
import gzip
import json
import os
from pathlib import Path
import subprocess
from export_regular_clicks import ROOT, VERSIONS, digest, encoded, original_modern_classpath
from export_item_properties import COMPILER


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('downloads', 'modern-classpath-file', 'runtime-output'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--normalize-only', action='store_true')
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    base = args.runtime_output.resolve()
    base.mkdir(parents=True, exist_ok=True)
    cp = os.pathsep.join(str(Path(p).resolve()) for p in args.modern_classpath_file.read_text().strip().split(os.pathsep))
    sources = ['scripts/' + name + '.java' for name in ('ExportInventoryTransfers', 'ExportItemComponents', 'ExportItemProperties', 'ExportRecipeDisplays', 'ExportRecipeGhosts')]
    requests = 'data/client_api/recipe_ghost_requests.json'
    named = base / 'named-sources'
    named.mkdir(exist_ok=True)
    for source in sources:
        (named / Path(source).name).write_text('package voxrig.oracle;\n' + (ROOT / source).read_text())
    compiler = base / 'CompileOwnTool.java'
    compiler.write_text(COMPILER)
    classes = base / 'own-classes'
    if not args.normalize_only:
        with (base / 'compile.log').open('w') as log:
            subprocess.run(['java', '-Xmx512M', '-XX:ActiveProcessorCount=1', str(compiler), cp, str(classes), *(str(named / Path(s).name) for s in sources)], cwd=base, stdout=log, stderr=subprocess.STDOUT, check=True)
    outputs, runs = {}, []
    for version, (jar_sha1, mapping_sha256) in VERSIONS.items():
        jar = (args.downloads / (version + '-server.jar')).resolve()
        assert digest(jar.read_bytes(), 'sha1') == jar_sha1
        assert digest((args.downloads / (version + '-server-mappings.txt')).read_bytes()) == mapping_sha256
        hashes = original_modern_classpath(jar, cp) if version == '1.21.11' else {digest(jar.read_bytes()): jar.name}
        raw = base / (version + '-raw.json')
        if not args.normalize_only:
            with (base / (version + '-run.log')).open('w') as log:
                subprocess.run(['java', '-Xmx512M', '-XX:ActiveProcessorCount=1', '-cp', str(classes) + os.pathsep + (str(jar) if version == '1.16.1' else cp), 'voxrig.oracle.ExportRecipeGhosts', version, str(ROOT / requests), str(raw)], cwd=base, stdout=log, stderr=subprocess.STDOUT, check=True)
        data = json.loads(raw.read_text())
        assert data['version'] == version
        name = 'data/client_api/recipe_ghost_cases-' + version + '.json.gz'
        outputs[name] = gzip.compress(encoded(data, compact=True), mtime=0)
        runs.append({'version': version, 'original_server_jar_sha1': jar_sha1, 'mappings_sha256': mapping_sha256, 'original_classpath_entries_sha256': hashes, 'raw_output_sha256': digest(raw.read_bytes()), 'cases': len(data['cases']), 'files_sha256': {name: digest(outputs[name])}})
    outputs['data/client_api/recipe_ghost_source.json'] = encoded({'schema': 1, 'authority': 'Unmodified original ghost-response constructors and stream codecs; legacy original protocol registration. No player/world scaffolding.', 'scope': 'Response codec only; no UI admission, request causation, inventory or crafted-output authority. Modern response has display and container ID, no recipe ID.', 'generators_sha256': {s: digest((ROOT / s).read_bytes()) for s in sources + ['scripts/export_recipe_ghost.py']}, 'requests_sha256': {requests: digest((ROOT / requests).read_bytes())}, 'generated_compiler_sha256': digest(COMPILER.encode()), 'runs': runs})
    for name, value in outputs.items():
        path = ROOT / name
        if args.check:
            assert path.read_bytes() == value, name
        else:
            path.write_bytes(value)
    print('Original recipe ghosts verified' if args.check else 'Original recipe ghosts generated', [(r['version'], r['cases']) for r in runs])


if __name__ == '__main__':
    main()
