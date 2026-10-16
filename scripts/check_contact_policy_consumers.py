#!/usr/bin/env python3
"""Check table requirements against independent Java factories and fixed oracles.

Does not modify the Java harness, Rust policy, or any fixture. With --rerun-output
also compare an original-jar owner run (written elsewhere) by (scene, seed).
"""
import argparse
import gzip
import hashlib
import json
import re
from pathlib import Path

from contact_policy_export import ROOT, table_export, targets


def load(path):
    return json.loads(gzip.decompress(path.read_bytes()))


def schedules(data):
    rows = {(row['scene'], row['seed']): row for row in data['results']}
    if len(rows) != len(data['results']):
        raise ValueError('duplicate scenario ID (scene, seed)')
    return rows


def check_pass_through(policy):
    # Independent decision boundary from Issue #128 option 3. Qualification
    # evidence and numerical scenario requirements are checked separately.
    for version, data in policy['versions'].items():
        for name, row in data['types'].items():
            for lane in ['OwnedWalking', 'OwnedGlide', 'OwnedBoat']:
                profile = row['lanes'][lane]
                expected = 'Refuse' if profile['modeled'] else 'PassThrough'
                if profile['unqualified'] != expected:
                    raise ValueError(f'{version}/{name}/{lane}: unqualified fallback drift')
    for name, profile in policy['packet_profiles'].items():
        if profile['default'] != 'Refuse' or profile['unproven_callbacks'] != 'PassThrough':
            raise ValueError(f'{name}: callback/family fallback drift')
        for rule in profile['exact']:
            if not rule['packet'].startswith(('Status(', 'Animation(')):
                raise ValueError(f'{name}: callback proof outside status/animation')
            if rule['decision'] != 'Irrelevant(PacketAudit)':
                raise ValueError(f'{name}: finite callback proof relabeled')
    print('pass-through: owned unqualified fallbacks and callback/family boundaries match')


def check_policy(policy, fixture):
    check_pass_through(policy)
    if set(fixture['versions']) != set(policy['versions']):
        raise ValueError('owner fixture version set differs from table')
    evidence = json.loads((ROOT / 'docs/evidence/contact-capabilities-20261015.json').read_text())
    for version, data in fixture['versions'].items():
        # Independent fixed corpus size and IDs; changing policy cannot prune it.
        seed_count = 72 if version == '1.16.1' else 96
        rows = schedules(data)
        expected_ids = {(scene, seed) for scene in ['walking', 'boat'] for seed in range(seed_count)}
        if set(rows) != expected_ids or any(len(row['steps']) != 12 for row in rows.values()):
            raise ValueError(f'{version}: fixed numerical scenario corpus changed')
        required = set().union(*(targets(policy, version, scenario)
                                for scenario in ['Living', 'Chicken', 'RemotePlayer']))
        types = policy['versions'][version]['types']
        modeled = {name for name, row in types.items() if row['lanes']['OwnedWalking']['modeled']}
        if required != modeled:
            raise ValueError(f'{version}: numerical scenario requirements/modeling differ')
        excluded = targets(policy, version, 'Excluded')
        probes = {row['type'].removeprefix('minecraft:') for row in data['exclusions']}
        irrelevant = {name for name, row in types.items()
                      if row['classifications']['walking'] == 'ProvenIrrelevant'}
        if excluded != irrelevant or probes & irrelevant != excluded:
            raise ValueError(f'{version}: exclusion probe/requirement sets differ')
        source = ROOT / 'scripts/entity_owner_oracle' / version / 'EntityTickOracle.java'
        java = source.read_text()
        if hashlib.sha256(source.read_bytes()).hexdigest() != data['harness_sha256']:
            raise ValueError(f'{version}: independent Java harness binding changed')
        # Every explicit spawn switch arm (including the default cow) must have
        # its own constructor. Original EntityType constants identify species;
        # RemotePlayer has a distinct native client constructor.
        spawn = java.split('static void spawn(', 1)[1].split('int fresh=', 1)[0]
        classes = set(re.findall(r'new (\w+)\(', spawn))
        factory_types = set()
        for cls in classes:
            constructor = re.search(r'\b' + cls + r'\([^\n]*?\)\{super\(([^\n]*?)\);', java)
            if not constructor:
                raise ValueError(f'{version}: missing factory constructor {cls}')
            native = re.search(r'EntityType\.(\w+)', constructor[1])
            if native:
                factory_types.add(native[1].lower())
            elif re.search(r'class ' + cls + r' extends net\.minecraft\.client\.player\.RemotePlayer', java):
                factory_types.add('player')
            else:
                raise ValueError(f'{version}: unresolved factory {cls}')
        observed = set()
        for scene in ['walking', 'boat']:
            scene_types = set()
            covered = set()
            for (row_scene, seed), row in rows.items():
                if row_scene != scene:
                    continue
                spawned = {action['type_name'].removeprefix('minecraft:')
                           for step in row['steps'] for action in step['actions'] if action['op'] == 'spawn'}
                if len(spawned) != 1:
                    raise ValueError(f'{version}/{scene}/{seed}: expected one actor species')
                scene_types.update(spawned)
                covered.update(spawned & required)
            if covered != required:
                raise ValueError(f'{version}/{scene}: missing required numerical actors {required - covered}')
            if scene_types != factory_types:
                raise ValueError(f'{version}/{scene}: Java factory/corpus sets differ: {scene_types ^ factory_types}')
            observed.update(scene_types)
        if factory_types & required != required:
            raise ValueError(f'{version}: missing required factories')
        # The raw observations deliberately include Unknown species. Keep them
        # and their original numerical values, without admitting them in Rust.
        if not observed <= set(evidence['versions'][version]['types']):
            raise ValueError(f'{version}: original actors missing from method-owner evidence')
        print(f'{version}: {len(required)} required numerical actors; {len(factory_types)} Java factories; '
              f'{len(rows)} fixed scenarios match')


def compare_rerun(fixture, rerun):
    if set(fixture['versions']) != set(rerun['versions']):
        raise ValueError('original rerun version sets differ')
    for version, original in fixture['versions'].items():
        actual = rerun['versions'][version]
        if schedules(original) != schedules(actual):
            raise ValueError(f'{version}: original rerun differs by scenario ID or numerical bytes')
        if original != actual:
            raise ValueError(f'{version}: original rerun provenance/exclusions/raw hash differ')
        print(f'{version}: original rerun scenario values, exclusions and raw output SHA-256 identical')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true', required=True)
    parser.add_argument('--rerun-output', type=Path)
    args = parser.parse_args()
    fixture = load(ROOT / 'data/client_api/entity_owner_oracle.json.gz')
    check_policy(table_export(), fixture)
    if args.rerun_output:
        compare_rerun(fixture, load(args.rerun_output))


if __name__ == '__main__':
    main()
