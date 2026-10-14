#!/usr/bin/env python3
"""Execute unchanged legacy Level queries and modern EntitySectionStorage queries.

Membership updates are explicit harness stages, not native ClientLevel ticks.
Only our source, numeric results and input/artifact hashes are published.
"""
import argparse
import gzip
import json
import os
from pathlib import Path
from run_entity_pose_oracle import mappings, member
from movement_oracle.run import classpath, digest, run, VERSIONS, SPECIAL_SOURCE_SHA256, PROPERTIES

REPO = Path(__file__).resolve().parents[1]


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--downloads', type=Path, required=True)
    p.add_argument('--work', type=Path, required=True)
    p.add_argument('--output', type=Path, default=REPO/'data/client_api/entity_query_oracle.json.gz')
    args = p.parse_args()
    args.work.mkdir(parents=True, exist_ok=True)
    tool = (args.downloads/'SpecialSource-1.11.4-shaded.jar').resolve()
    assert digest(tool.read_bytes()) == SPECIAL_SOURCE_SHA256
    result = dict(schema=1, authority='Unchanged legacy Level.getEntities/LevelChunk lists and modern EntitySectionStorage accessible-section query, with original selector. Registered official worlds; explicit model membership stages, not native packet scheduling or ClientLevel ticks.', versions={})
    for port, (version, (sha, msha, _)) in enumerate(VERSIONS.items(), start=25750):
        work = (args.work/version).resolve()
        (work/'classes').mkdir(parents=True, exist_ok=True)
        jar = (args.downloads/f'{version}-server.jar').resolve()
        mapping = (args.downloads/f'{version}-server-mappings.txt').resolve()
        assert digest(jar.read_bytes(), 'sha1') == sha and digest(mapping.read_bytes(), 'sha1') == msha
        game, cp = classpath(version, jar, work)
        named = work/'named.jar'
        run(['java', '-jar', str(tool), '--in-jar', str(game), '--out-jar', str(named), '--srg-in', str(mapping)], work, work/'remap.log')
        source = REPO/'scripts/entity_query_oracle'/version/'EntityQueryOracle.java'
        run(['javac', '-nowarn', '--release', '21', '-d', str(work/'classes'), '-cp', os.pathsep.join(str(p) for p in [named, *cp[1:]]), str(source)], work, work/'javac.log')
        run(['jar', 'cf', str(work/'harness-named.jar'), '-C', str(work/'classes'), 'voxrig'], work, work/'jar.log')
        run(['java', '-cp', f'{tool}:{named}', 'net.md_5.specialsource.SpecialSource', '--live', '--in-jar', str(work/'harness-named.jar'), '--out-jar', str(work/'harness.jar'), '--srg-in', str(mapping), '--reverse'], work, work/'unmap.log')
        server = work/'server'
        server.mkdir(exist_ok=True)
        (server/'eula.txt').write_text('eula=true\n')
        (server/'server.properties').write_text(PROPERTIES.format(port=port, level_type='flat' if version=='1.16.1' else 'minecraft\\:flat'))
        output = work/'original-results.json'
        extra = [member(mappings(mapping), 'net.minecraft.world.level.Level', ' isClientSide ->')]
        run(['java', '-Xmx1G', '--add-opens', 'java.base/java.lang=ALL-UNNAMED', '-cp', os.pathsep.join(str(p) for p in [*cp, work/'harness.jar']), 'voxrig.oracle.EntityQueryOracle', str(output), *extra], server, work/'server.log')
        rows = json.loads(output.read_text())
        assert len(rows) == 48 and sum(len(r['steps']) for r in rows) == 288
        result['versions'][version] = dict(original_server_jar_sha1=sha, mappings_sha1=msha, original_output_sha256=digest(output.read_bytes()), harness_sha256=digest(source.read_bytes()), results=rows)
    result['runner_sha256'] = digest(Path(__file__).read_bytes())
    args.output.write_bytes(gzip.compress((json.dumps(result, separators=(',', ':'))+'\n').encode(), mtime=0))
    print('Original spatial queries:', sum(len(r['steps']) for v in result['versions'].values() for r in v['results']))


if __name__ == '__main__':
    main()
