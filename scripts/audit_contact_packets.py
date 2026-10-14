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


def digest(path):
    return hashlib.sha1(path.read_bytes()).hexdigest()


def method(code, name):
    match = re.search(r'^  [^\n]* ' + name + r'\([^\n]*\);\n', code, re.M)
    assert match, name
    end = re.search(r'\n  [^ \n]', code[match.end():])
    return code[match.start():match.end() + end.start()] if end else code[match.start():]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--downloads', type=Path, required=True)
    parser.add_argument('--work', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
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
    args.output.write_text(json.dumps(result, indent=2) + '\n')


if __name__ == '__main__':
    main()
