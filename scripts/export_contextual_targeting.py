#!/usr/bin/env python3
"""Run the original 1.21.11 dimensions/view/clip methods without starting a server."""
import argparse
import gzip
import json
import os
import subprocess
from pathlib import Path
from export_regular_clicks import ROOT, digest, encoded, original_modern_classpath
from export_item_properties import COMPILER


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('jar', 'mappings', 'modern-classpath-file', 'work'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    jar, mappings, work = args.jar.resolve(), args.mappings.resolve(), args.work.resolve()
    assert digest(jar.read_bytes(), 'sha1') == '64bb6d763bed0a9f1d632ec347938594144943ed'
    assert digest(mappings.read_bytes(), 'sha1') == '5621e9253f05fd57872bbe7f8ddf5f9a7d525955'
    cp = os.pathsep.join(str(Path(p).resolve()) for p in args.modern_classpath_file.read_text().strip().split(os.pathsep))
    original = original_modern_classpath(jar, cp)
    sources = ['scripts/ExportStorageOutlines.java', 'scripts/ExportContextualTargeting.java']
    folder = work / 'sources'
    folder.mkdir(parents=True, exist_ok=True)
    for source in sources:
        (folder / Path(source).name).write_text('package voxrig.oracle;\n' + (ROOT / source).read_text())
    compiler = work / 'CompileOwnTool.java'
    compiler.write_text(COMPILER)
    classes = work / 'classes'
    raw = work / 'raw.json'
    commands = [
        ['java', '-Xmx512M', '-XX:ActiveProcessorCount=1', str(compiler), cp, str(classes),
         *(str(folder / Path(p).name) for p in sources)],
        ['java', '-Xmx512M', '-XX:ActiveProcessorCount=1', '-cp', str(classes) + os.pathsep + cp,
         'voxrig.oracle.ExportContextualTargeting', str(raw)],
    ]
    for i, command in enumerate(commands):
        with (work / f'run-{i}.log').open('w') as log:
            subprocess.run(command, cwd=work, stdout=log, stderr=subprocess.STDOUT, check=True)
    data = json.loads(raw.read_bytes())
    assert len(data) == 1344
    fixture = gzip.compress(encoded(data, compact=True), mtime=0)
    source = encoded(dict(
        authority='Unmodified official Avatar POSES/EntityDimensions.scale/makeBoundingBox, Entity.calculateViewVector and BlockGetter.clip OUTLINE/NONE with absent CollisionContext; state-only geometry.',
        scope='Numerical dimensions/eye and static first hit only. Inside fixtures deliberately bypass ground admission. Not a graphical client clock, entity/fluid selection, replaceability or action permission.',
        original_server_jar_sha1=digest(jar.read_bytes(), 'sha1'),
        mappings_sha1=digest(mappings.read_bytes(), 'sha1'),
        original_classpath_sha256=original,
        generators_sha256={p: digest((ROOT / p).read_bytes()) for p in sources + ['scripts/export_contextual_targeting.py']},
        generated_compiler_sha256=digest(COMPILER.encode()),
        fixture_sha256=digest(fixture), cases=len(data),
    ))
    for path, value in [('data/client_api/contextual_targeting.json.gz', fixture),
                        ('data/client_api/contextual_targeting_source.json', source)]:
        if args.check:
            assert (ROOT / path).read_bytes() == value, path
        else:
            (ROOT / path).write_bytes(value)
    print('1344 original contextual dimension/clip cases verified')


if __name__ == '__main__':
    main()
