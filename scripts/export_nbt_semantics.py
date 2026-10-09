#!/usr/bin/env python3
"""Pin original NBT decoding, equality and modern persistent CRC32C facts sequentially."""
import argparse
import os
from pathlib import Path
import subprocess
import json
from export_regular_clicks import ROOT, VERSIONS, digest, encoded, original_modern_classpath

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--downloads', type=Path, required=True)
    parser.add_argument('--modern-classpath-file', type=Path, required=True)
    parser.add_argument('--runtime-output', type=Path, required=True)
    parser.add_argument('--normalize-only', action='store_true')
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    base = args.runtime_output.resolve(); base.mkdir(parents=True, exist_ok=True)
    cp = os.pathsep.join(str(Path(p).resolve()) for p in args.modern_classpath_file.read_text().strip().split(os.pathsep))
    requests = 'data/client_api/nbt_semantics_requests.json'
    source = 'scripts/ExportNbtSemantics.java'
    outputs, runs = {}, []
    for version, (jar_sha1, mapping_sha256) in VERSIONS.items():
        jar = (args.downloads / (version + '-server.jar')).resolve()
        mapping = args.downloads / (version + '-server-mappings.txt')
        if digest(jar.read_bytes(), 'sha1') != jar_sha1 or digest(mapping.read_bytes()) != mapping_sha256:
            raise SystemExit('original inputs differ: ' + version)
        hashes = original_modern_classpath(jar, cp) if version == '1.21.11' else {digest(jar.read_bytes()): jar.name}
        raw = base / (version + '-raw.json')
        if not args.normalize_only:
            with (base / (version + '-run.log')).open('w') as log:
                subprocess.run(['java', '-Xmx512M', '-XX:ActiveProcessorCount=1', '-cp', str(jar) if version == '1.16.1' else cp,
                                str(ROOT / source), version, str(ROOT / requests), str(raw)],
                               cwd=base, stdout=log, stderr=subprocess.STDOUT, check=True)
        data = json.loads(raw.read_text()); assert data['version'] == version
        file = 'data/client_api/nbt_semantics-' + version + '.json'
        outputs[file] = encoded(data)
        runs.append({'version':version, 'original_server_jar_sha1':jar_sha1, 'mappings_sha256':mapping_sha256,
                     'original_classpath_entries_sha256':hashes, 'raw_output_sha256':digest(raw.read_bytes()),
                     'values':len(data['values']), 'pairs':len(data['pairs']), 'native_failures':len(data['failures']),
                     'files_sha256':{file:digest(outputs[file])}})
    outputs['data/client_api/nbt_semantics_source.json'] = encoded({
        'schema':1, 'authority':'Unmodified original NbtIo decode/write, native Tag.equals and modern CompoundTag.CODEC encodeStart(HashOps.CRC32C_INSTANCE).',
        'scope':'NBT value meaning and pure persistent value hashes. Not general component/prototype/reference semantics, cached server hash behavior, data-bearing gameplay or live receipt proof.',
        'generators_sha256':{p:digest((ROOT / p).read_bytes()) for p in [source,requests,'scripts/export_nbt_semantics.py','scripts/export_regular_clicks.py']},
        'runs':runs})
    for name, data in outputs.items():
        path = ROOT / name
        if args.check:
            if path.read_bytes() != data: raise SystemExit('NBT facts differ: ' + name)
        else: path.write_bytes(data)
    print('Original NBT semantics verified' if args.check else 'Original NBT semantics generated')
    for run in runs: print(run['version'], 'values/pairs/native failures', run['values'],run['pairs'],run['native_failures'])

if __name__ == '__main__': main()
