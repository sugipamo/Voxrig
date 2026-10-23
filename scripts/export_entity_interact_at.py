#!/usr/bin/env python3
"""Construct/roundtrip INTERACT_AT with the unchanged official 1.21.11 codec."""
import argparse
import json
import os
from pathlib import Path
import subprocess
from export_regular_clicks import ROOT, digest, encoded, original_modern_classpath


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
    work.mkdir(parents=True, exist_ok=True)
    raw = work / 'raw.json'
    source = 'scripts/ExportEntityInteractAt.java'
    with (work / 'codec.log').open('w') as log:
        subprocess.run(['java', '-Xmx512M', '-XX:ActiveProcessorCount=1', '-cp', cp,
                        str(ROOT / source), str(raw)], cwd=work, stdout=log, stderr=subprocess.STDOUT, check=True)
    rows = json.loads(raw.read_bytes())
    assert len(rows) == 96
    fixture = encoded(rows)
    manifest = encoded(dict(
        authority='Unmodified official ServerboundInteractPacket constructor, write and read/write roundtrip; no server or handlers.',
        scope='INTERACT_AT field order, float bits, hand, secondary action and VarInt boundaries. Not admission, reach or game outcome.',
        original_server_jar_sha1=digest(jar.read_bytes(), 'sha1'),
        mappings_sha1=digest(mappings.read_bytes(), 'sha1'),
        original_classpath_sha256=original,
        generators_sha256={p: digest((ROOT / p).read_bytes()) for p in [source, 'scripts/export_entity_interact_at.py']},
        fixture_sha256=digest(fixture), cases=len(rows)))
    for path, value in [('data/client_api/entity_interact_at_packets.json', fixture),
                        ('data/client_api/entity_interact_at_source.json', manifest)]:
        if args.check:
            assert (ROOT / path).read_bytes() == value, path
        else:
            (ROOT / path).write_bytes(value)
    print('96 original INTERACT_AT codec cases verified')


if __name__ == '__main__':
    main()
