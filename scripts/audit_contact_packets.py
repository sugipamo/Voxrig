#!/usr/bin/env python3
"""Record original-client provenance and method fingerprints for the step A audit.

Use named clients produced by run_entity_owner_oracle.py. Published output
contains hashes, never redistributed native code. See docs/contact-packet-audit.md
for the required manual side-effect and field-use inspection.
"""
import argparse
import hashlib
import json
import re
import subprocess
from pathlib import Path

from run_entity_owner_oracle import CLIENT
from contact_policy_export import ROOT, packet_targets, table_export


def digest(path):
    return hashlib.sha1(path.read_bytes()).hexdigest()


def method(code, name):
    match = re.search(r'^  [^\n]* ' + name + r'\([^\n]*\);\n', code, re.M)
    assert match, name
    end = re.search(r'\n  [^ \n]', code[match.end():])
    return code[match.start():match.end() + end.start()] if end else code[match.start():]


def check_policy(policy, evidence):
    # Independent manual proof inventory from this audit, not Rust generation.
    for version, data in policy['versions'].items():
        modeled = {name for name, row in data['types'].items()
                   if row['lanes']['OwnedWalking']['modeled']}
        expected = {packet: modeled for packet in ['Status(35)', 'Animation(4)', 'Animation(5)']}
        expected['Status(10)'] = {'sheep'}
        if packet_targets(policy, version) != expected:
            raise ValueError(f'{version}: table allows differ from manual packet proof inventory')
        row = evidence[version]
        if (row['original_client_sha1'], row['mappings_sha1']) != CLIENT[version]:
            raise ValueError(f'{version}: packet audit original provenance mismatch')
        if not row['javap_method_sha256']:
            raise ValueError(f'{version}: packet fingerprints missing')
    if set(evidence) != set(policy['versions']):
        raise ValueError('packet audit version sets differ')
    print('packet audit: exact opcode/target sets match const table')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--downloads', type=Path)
    parser.add_argument('--work', type=Path)
    parser.add_argument('--output', type=Path, default=ROOT / 'data/client_api/contact_packet_audit.json')
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    policy = table_export()
    if args.check:
        check_policy(policy, json.loads(args.output.read_text()))
        print('jar-dependent collection skipped in --check; checked-in fingerprints retained')
        return
    if args.downloads is None or args.work is None:
        parser.error('--downloads and --work are required for original-jar collection')
    result = {}
    for version, (jar_hash, mapping_hash) in CLIENT.items():
        jar = args.downloads / f'{version}-client.jar'
        mapping = args.downloads / f'{version}-client-mappings.txt'
        assert digest(jar) == jar_hash
        assert digest(mapping) == mapping_hash
        classes = {
            'net.minecraft.client.particle.ParticleEngine': ['createTrackingEmitter'],
            'net.minecraft.client.particle.TrackingEmitter': ['tick'],
            'net.minecraft.client.multiplayer.ClientPacketListener': ['handleAnimate', 'handleEntityEvent'],
            'net.minecraft.world.entity.animal.' + ('sheep.' if version == '1.21.11' else '') + 'Sheep': ['handleEntityEvent', 'aiStep', 'getHeadEatPositionScale', 'getHeadEatAngleScale'],
        }
        fingerprints = {}
        for cls, methods in classes.items():
            code = subprocess.check_output(['javap', '-c', '-p', '-classpath', str(args.work / version / 'named.jar'), cls], text=True)
            for name in methods:
                body = method(code, name)
                fingerprints[cls + '.' + name] = hashlib.sha256(body.encode()).hexdigest()
            if cls.endswith('Sheep'):
                body = method(code, 'handleEntityEvent')
                assert 'bipush        10' in body and 'bipush        40' in body
                assert 'Field eatAnimationTick:I' in body
        result[version] = {'original_client_sha1': jar_hash, 'mappings_sha1': mapping_hash, 'javap_method_sha256': fingerprints}
    check_policy(policy, result)
    args.output.write_text(json.dumps(result, indent=2) + '\n')


if __name__ == '__main__':
    main()
