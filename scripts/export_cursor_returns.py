#!/usr/bin/env python3
"""Export original left PICKUP default-item return/take/merge and codec facts.
One original native JVM at a time, 512 MiB/CPU 1. Own helper classes use a named
package to coexist with the signed modern native unnamed package. Game JARs,
methods, constructors and codecs remain unchanged; contexts are primitive only.
"""
import argparse, gzip, json, os, subprocess
from pathlib import Path
from export_regular_clicks import ROOT, VERSIONS, digest, encoded, original_modern_classpath

COMPILER = '''import javax.tools.ToolProvider;
public class CompileOwnTool {
 public static void main(String[] args) {
  int result = ToolProvider.getSystemJavaCompiler().run(null, null, null,
    "-proc:none", "-cp", args[0], "-d", args[1], args[2], args[3]);
  if (result != 0) throw new IllegalStateException("own tooling compilation failed");
 }
}
'''

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--downloads', type=Path, required=True)
    parser.add_argument('--modern-classpath-file', type=Path, required=True)
    parser.add_argument('--runtime-output', type=Path, required=True)
    parser.add_argument('--normalize-only', action='store_true')
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    base = args.runtime_output.resolve()
    base.mkdir(parents=True, exist_ok=True)
    cp = os.pathsep.join(str(Path(p).resolve()) for p in args.modern_classpath_file.read_text().strip().split(os.pathsep))
    outputs, runs = {}, []
    own_sources = ['scripts/ExportInventoryTransfers.java', 'scripts/ExportCursorReturns.java']
    named = base / 'named-sources'
    named.mkdir(exist_ok=True)
    for file in own_sources:
        (named / Path(file).name).write_text('package voxrig.oracle;\n' + (ROOT / file).read_text())
    compiler = base / 'CompileOwnTool.java'
    compiler.write_text(COMPILER)
    for version, (jar_sha1, mapping_sha256) in VERSIONS.items():
        jar = (args.downloads / f'{version}-server.jar').resolve()
        mapping = args.downloads / f'{version}-server-mappings.txt'
        if digest(jar.read_bytes(), 'sha1') != jar_sha1 or digest(mapping.read_bytes()) != mapping_sha256:
            raise SystemExit('original native inputs differ: ' + version)
        classpath = str(jar) if version == '1.16.1' else cp
        hashes = {digest(jar.read_bytes()): jar.name} if version == '1.16.1' else original_modern_classpath(jar, cp)
        raw_path = base / f'{version}-raw.json'
        if not args.normalize_only:
            classes = base / f'{version}-own-classes'
            with (base / f'{version}-compile.log').open('w') as log:
                subprocess.run(['java', '-Xmx512M', '-XX:ActiveProcessorCount=1', str(compiler), classpath,
                    str(classes), *(str(named / Path(p).name) for p in own_sources)], stdout=log, stderr=subprocess.STDOUT, check=True)
            with (base / f'{version}-run.log').open('w') as log:
                subprocess.run(['java', '-Xmx512M', '-XX:ActiveProcessorCount=1', '-cp', str(classes) + os.pathsep + classpath,
                    'voxrig.oracle.ExportCursorReturns', version, str(raw_path)], stdout=log, stderr=subprocess.STDOUT, check=True)
        raw = json.loads(raw_path.read_text())
        grouped = {}
        for case in raw['cases']:
            grouped.setdefault(case['item'], []).append(case)
        expected = 974 if version == '1.16.1' else 1504
        if raw['version'] != version or len(grouped) != expected or len(raw['packets']) != expected * 2:
            raise SystemExit('cursor return coverage differs: ' + version)
        items = []
        for name, cases in grouped.items():
            sample = next(c for c in cases if c['source_before']['count'] == 0 and c['cursor_before']['count'] == 1)
            items.append({'name': name, 'native_id': sample['native_id'], 'maximum': sample['maximum'],
                'default_legacy_nbt': sample['cursor_before'].get('nbt'), 'cases': len(cases)})
        prefix = 'data/client_api/cursor_return_'
        paths = [prefix + f'profiles-{version}.json', prefix + f'cases-{version}.json.gz', prefix + f'packets-{version}.json']
        outputs[paths[0]] = encoded({'version': version, 'items': items, 'excluded_nonempty_override_cases': raw['excluded_nonempty_override_cases']})
        outputs[paths[1]] = gzip.compress(encoded({'version': version, 'cases': raw['cases']}, compact=True), mtime=0)
        outputs[paths[2]] = encoded(raw['packets'])
        runs.append({'version': version, 'original_server_jar_sha1': jar_sha1, 'mappings_sha256': mapping_sha256,
            'original_classpath_entries_sha256': hashes, 'raw_output_sha256': digest(raw_path.read_bytes()),
            'native_cases': len(raw['cases']), 'default_items': expected, 'codec_roundtrips': len(raw['packets']),
            'files_sha256': {p: digest(outputs[p]) for p in paths}})
    outputs['data/client_api/cursor_return_source.json'] = encoded({'schema': 1,
        'authority': 'Actual unchanged native left PICKUP and exact legacy NBT/default modern hash codecs on pinned official JARs; no game method bodies redistributed.',
        'scope': 'Default-item take into Empty cursor, deposit into Empty player main/hotbar, ordinary same-stack merge/full cases. Nonempty native item overrides (bundle insertion) are explicitly excluded, not fabricated as no-ops; close preflight does not select full/mismatched stacks. After each modern case both resulting native stacks must remain default for original hash encoder.',
        'context': 'Original Transfer primitive bootstrap reused, unspawned native ServerPlayer/Inventory/EntityEquipment/native default features. Named package applies to own wrappers only; original signed JAR classes/methods remain unchanged. Primitive facts are not network/mode/session/ownership/close completion proof.',
        'generators_sha256': {p: digest((ROOT / p).read_bytes()) for p in own_sources + ['scripts/export_cursor_returns.py', 'scripts/export_regular_clicks.py']},
        'generated_compiler_sha256': digest(COMPILER.encode()), 'runs': runs})
    for path, data in outputs.items():
        target = ROOT / path
        if args.check:
            if target.read_bytes() != data:
                raise SystemExit('generated cursor return evidence differs: ' + path)
        else:
            target.write_bytes(data)
    print('native cursor return evidence verified' if args.check else 'native cursor return evidence generated')

if __name__ == '__main__':
    main()
