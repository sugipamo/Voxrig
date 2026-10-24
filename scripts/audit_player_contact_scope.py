#!/usr/bin/env python3
"""Bind the manual model-origin/group exclusion review to original bytecode.

Requires the original client jars/mappings and freshly remapped named jars from
scripts/audit_contact_envelopes.py. Emits method owners/hashes, never source code.
"""
import argparse
import json
import functools
import re
import subprocess
from pathlib import Path
from audit_contact_capabilities import CLIENT, ROOT, digest, audit
from audit_contact_envelopes import methods

TYPES = ['chicken', 'cow', 'creeper', 'giant', 'mooshroom', 'ocelot', 'pig',
         'rabbit', 'sheep', 'silverfish', 'villager', 'wandering_trader', 'zombie', 'player']
SOURCES = ['src/client/entity/scope.rs', 'src/client/entity/owned_contacts.rs',
           'src/client/entity/tick_order.rs', 'src/client/entity/query.rs',
           'src/client/entity/contact_policy/mod.rs',
           'src/client/entity/contact_policy/table.rs', 'src/client/entity/contact_policy/locality.rs',
           'src/client/entity/contact_capabilities.rs',
           'docs/evidence/contact-capabilities-20261014.json',
           'docs/evidence/contact-envelopes-20261015.json',
           'src/movement_refusal.rs', 'docs/player-contact-scope.md',
           'scripts/audit_player_contact_scope.py']


@functools.lru_cache(maxsize=None)
def raw(jar, cls):
    return subprocess.check_output(['javap', '-p', '-classpath', str(jar), cls], text=True)


def resolve(jar, cls, method):
    text = raw(jar, cls)
    if re.search(r' ' + re.escape(method) + r'\(', text):
        return cls
    header = next((line for line in text.splitlines() if re.search(r'\b(class|interface)\b', line)), '')
    parent = re.search(r' extends ([\w.$]+)', header)
    return resolve(jar, parent[1], method) if parent else None


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--downloads', type=Path)
    p.add_argument('--named', type=Path)
    p.add_argument('--output', type=Path, default=ROOT/'docs/evidence/player-contact-scope-20261024.json')
    p.add_argument('--check', action='store_true')
    args = p.parse_args()
    if args.check:
        evidence = json.loads(args.output.read_text())
        for source, expected in evidence['source_sha256'].items():
            assert digest(ROOT/source) == expected, source
        for version, (jar_sha, mappings_sha) in CLIENT.items():
            data = evidence['versions'][version]
            assert data['original_client_jar_sha1'] == jar_sha
            assert data['client_mappings_sha1'] == mappings_sha
            assert set(data['types']) == set(TYPES)
        print('Player scope review/source bindings and version/type domains match')
        return
    if args.downloads is None or args.named is None:
        p.error('--downloads and --named are required for original-jar collection')
    evidence = dict(schema=1, contract='decision A: received model origin and explicit numerical ticks; no real-time server proof',
        review='docs/player-contact-scope.md', versions={})
    for version, (jar_sha, mappings_sha) in CLIENT.items():
        assert digest(args.downloads/f'{version}-client.jar', 'sha1') == jar_sha
        assert digest(args.downloads/f'{version}-client-mappings.txt', 'sha1') == mappings_sha
        jar = args.named/version/'named.jar'
        rows = audit(jar)
        selected = {}
        roots = {'net.minecraft.world.entity.EntityType', 'net.minecraft.world.entity.EntityDimensions'}
        for name in TYPES:
            cls = rows[name]['class_name']
            roots.add(cls)
            selected[name] = {'class_name': cls}
            for method in ['rideTick', 'positionRider', 'getMyRidingOffset', 'getPassengersRidingOffset',
                           'getPassengerAttachmentPoint', 'getVehicleAttachmentPoint', 'getPassengerRidingPosition',
                           'isLocalInstanceAuthoritative', 'isEffectiveAi']:
                selected[name][method] = resolve(jar, cls, method)
        if version == '1.21.11':
            roots.update({'net.minecraft.world.entity.EntityAttachments', 'net.minecraft.world.entity.EntityAttachments$Builder',
                          'net.minecraft.world.entity.InterpolationHandler', 'net.minecraft.world.entity.InterpolationHandler$InterpolationData'})
        evidence['versions'][version] = dict(original_client_jar_sha1=jar_sha, client_mappings_sha1=mappings_sha,
            remapped_jar_sha256=digest(jar), types=selected, method_sha256=methods(jar, roots))
    evidence['existing_providers'] = {
        'no_contact_class_proof': 'existing version-qualified ProvenIrrelevant contact policy, whole groups only',
        'default_spider_envelope': 'existing envelope audit; unmounted only; all original revocations retained',
    }
    evidence['source_sha256'] = {source: digest(ROOT/source) for source in SOURCES}
    args.output.write_text(json.dumps(evidence, indent=2)+'\n')
    print('Recorded original method owners and fingerprints for both player scope versions')


if __name__ == '__main__':
    main()
