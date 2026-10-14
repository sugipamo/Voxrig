#!/usr/bin/env python3
"""Record contact method owners from the owner oracle's locally remapped clients.

Run run_entity_owner_oracle.py first with the same --downloads/--work inputs.
Only method ownership and verified original-input hashes are published, never
JARs, mappings or method bodies. Inspect species/ageable aiStep and metadata
callback bytecode locally as well: ownership alone cannot qualify wrappers.
"""
import argparse
import gzip
import json
import re
import subprocess
from pathlib import Path

from movement_oracle.run import digest
from run_entity_owner_oracle import CLIENT


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--downloads', type=Path, required=True)
    parser.add_argument('--work', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    result = {'schema': 1, 'versions': {}}
    for version, (jar_sha, mapping_sha) in CLIENT.items():
        assert digest((args.downloads / f'{version}-client.jar').read_bytes(), 'sha1') == jar_sha
        assert digest((args.downloads / f'{version}-client-mappings.txt').read_bytes(), 'sha1') == mapping_sha
        cache = {}

        def declaration(cls):
            if cls not in cache:
                raw = subprocess.check_output(['javap', '-p', '-classpath', str(args.work / version / 'named.jar'), cls], text=True)
                parent = re.search(r'^public (?:abstract )?class [^\n]*? extends ([\w.$]+)', raw, re.M)
                methods = set(re.findall(r' (\w+)\([^;]*\);', raw))
                cache[cls] = (parent.group(1) if parent else None, methods)
            return cache[cls]

        def owner(cls, method):
            parent, methods = declaration(cls)
            if method in methods:
                return cls
            return owner(parent, method) if parent else None

        types = {}
        for name, cls in [('sheep', 'net.minecraft.world.entity.animal.' + ('sheep.' if version == '1.21.11' else '') + 'Sheep'), ('creeper', 'net.minecraft.world.entity.monster.Creeper')]:
            types[name] = {method: owner(cls, method) for method in ['aiStep', 'pushEntities', 'doPush', 'push', 'isPushable', 'getDimensions', 'getDefaultDimensions', 'getAgeScale', 'isBaby', 'onSyncedDataUpdated']}
            assert all(types[name][method] == 'net.minecraft.world.entity.LivingEntity' for method in ['pushEntities', 'doPush', 'push', 'isPushable', 'getDimensions'])
        metadata_path = Path(__file__).resolve().parents[1] / 'data/client_api' / f'entity_data-{version}.json'
        metadata = json.loads(metadata_path.read_text())['entities']['minecraft:sheep']
        baby = next(row for row in metadata if row[1] == 'DATA_BABY_ID')
        assert baby[0] == ('AgableMob' if version == '1.16.1' else 'AgeableMob')
        assert baby[3:] == ['bool', False]
        result['versions'][version] = {'original_client_jar_sha1': jar_sha, 'client_mappings_sha1': mapping_sha, 'types': types,
            'sheep_baby_metadata': baby, 'entity_data_sha256': digest(metadata_path.read_bytes())}
    fixture_path = Path(__file__).resolve().parents[1] / 'data/client_api/entity_owner_oracle.json.gz'
    fixture = json.loads(gzip.decompress(fixture_path.read_bytes()))
    result['numerical_oracle'] = {
        'fixture_sha256': digest(fixture_path.read_bytes()),
        'runner_sha256': fixture['runner_sha256'],
        'schedules': sum(len(v['results']) for v in fixture['versions'].values()),
        'stages': sum(len(row['steps']) for v in fixture['versions'].values() for row in v['results']),
        'authority': fixture['authority'],
        'provenance': {v: {k: value for k, value in data.items() if k != 'results'} for v, data in fixture['versions'].items()},
    }
    result['runner_sha256'] = digest(Path(__file__).read_bytes())
    args.output.write_text(json.dumps(result, indent=2) + '\n')


if __name__ == '__main__':
    main()
