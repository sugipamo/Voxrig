#!/usr/bin/env python3
"""Reproduce Step B's finite spatial-proof inputs from verified original clients.

Semantic review is docs/contact-envelope-audit.md. Fingerprints bind that review
and the implementation to the original owners, not an ancestry qualification.
No bytecode or proprietary mappings are emitted.
"""
import argparse
import hashlib
import json
import re
import subprocess
from pathlib import Path

from audit_contact_capabilities import CLIENT, ROOT, audit, digest
from contact_policy_export import table_export, targets


def methods(jar, roots):
    result = {}
    todo = set(roots)
    while todo:
        cls = min(todo)
        todo.remove(cls)
        if cls in result or cls.startswith('java.'):
            continue
        raw = subprocess.check_output(
            ['javap', '-c', '-p', '-classpath', str(jar), cls], text=True)
        parent = re.search(r'(?:class|interface) [\w.$]+(?:<[^\n]*?>)? extends ([\w.$]+)', raw)
        if parent:
            todo.add(parent[1])
        result[cls] = {
            block.split('\n')[0].strip(): hashlib.sha256(block.encode()).hexdigest()
            for block in re.split(r'\n(?=  [^ ])', raw) if '    Code:' in block
        }
    return dict(sorted(result.items()))


def resolve(jar, cls, method):
    raw = subprocess.check_output(['javap', '-p', '-classpath', str(jar), cls], text=True)
    if re.search(r' ' + re.escape(method) + r'\(', raw):
        return cls
    parent = re.search(r' extends ([\w.$]+)', raw)
    return resolve(jar, parent[1], method) if parent else None


def check_policy(policy, evidence):
    if set(evidence['versions']) != set(policy['versions']):
        raise ValueError('envelope audit version sets differ')
    for version, data in evidence['versions'].items():
        required = targets(policy, version, 'SpiderEnvelope')
        if set(data['types']) != required:
            raise ValueError(f'{version}: envelope evidence/const-table target sets differ')
        locality = policy['locality_profiles']['DefaultSpiderEnvelope']
        # Independent proof-breaking receipt domain; all 512 sub-opcodes.
        revokes = {'Metadata', 'Equipment', 'Scale', 'Passengers'}
        revokes.update(f'Status({n})' for n in range(-128, 128))
        revokes.update(f'Animation({n})' for n in range(256))
        if set(locality['revokes']) != revokes:
            raise ValueError('envelope revocations differ from manual proof')
        eligible = {name for name, row in policy['versions'][version]['types'].items()
                    if row['lanes']['OwnedWalking']['locality'] == 'DefaultSpiderEnvelope'}
        if eligible != required:
            raise ValueError('envelope eligibility/scenario sets differ')
        if (data['original_client_jar_sha1'], data['client_mappings_sha1']) != CLIENT[version]:
            raise ValueError(f'{version}: envelope provenance mismatch')
    print('envelope audit: target sets and 516 proof-breaking receipts match const table')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--downloads', type=Path)
    parser.add_argument('--work', type=Path)
    parser.add_argument('--output', type=Path, default=ROOT / 'docs/evidence/contact-envelopes-20261015.json')
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    policy = table_export()
    if args.check:
        check_policy(policy, json.loads(args.output.read_text()))
        print('jar-dependent collection skipped in --check; checked-in fingerprints retained')
        return
    if args.downloads is None or args.work is None:
        parser.error('--downloads and --work are required for original-jar collection')
    result = {
        'schema': 1,
        'scope': 'Step B; finite default unmounted Spider/CaveSpider contact envelope only; no numerical body qualification',
        'rules': {
            'anchors': 'spawn, codec bases, all received interpolation targets and raw absolute endpoints, including X/Z placement clamped to +/-30 million; hull never shrinks',
            'shape': 'default original EntityType width/height; full horizontal width plus one block margin on every axis',
            'effects': 'inherited pair selector inflated by 0.2 horizontally; null/false solid collision; no client subclass nonlocal impulse',
            'callbacks': 'metadata, scale, equipment, status, animation or mount involvement removes finite proof for lifetime',
            'indirect': 'must miss every qualified remote current/target/shifted-target box and every fixed boat collider, not just own body',
            'tick_queries': 'own before/after hull, boat selector, every terrain cell visited by swept/step-up/noCollision queries',
            'fallback': 'no finite proof implies global typed Unsupported, regardless of distance',
        },
        'versions': {},
    }
    for version, (jar_sha, mappings_sha) in CLIENT.items():
        source = args.downloads / f'{version}-client.jar'
        mappings = args.downloads / f'{version}-client-mappings.txt'
        tool = args.downloads / 'SpecialSource-1.11.4-shaded.jar'
        assert digest(source, 'sha1') == jar_sha
        assert digest(mappings, 'sha1') == mappings_sha
        assert digest(tool) == 'e2cab24b1c12400ad73b15972bb21e4273a0dc7081c8b3c136ddfdd824c78518'
        named = args.work / version / 'named.jar'
        named.parent.mkdir(parents=True, exist_ok=True)
        subprocess.check_call(['java', '-jar', str(tool), '--in-jar', str(source),
                               '--out-jar', str(named), '--srg-in', str(mappings)],
                              stdout=subprocess.DEVNULL)
        rows = audit(named)
        types = {name: rows[name] for name in sorted(targets(policy, version, 'SpiderEnvelope'))}
        # Resolve separately on each original: no name/ancestry shortcut.
        for row in types.values():
            assert row['aiStep'] == 'net.minecraft.world.entity.monster.Monster'
            for method in ['pushEntities', 'doPush', 'isPushable']:
                assert row[method] == 'net.minecraft.world.entity.LivingEntity'
            if version == '1.16.1':
                assert row['getCollideBox'] == 'net.minecraft.world.entity.Entity'
                assert row['lerpTo'] == 'net.minecraft.world.entity.LivingEntity'
            else:
                assert row['canBeCollidedWith'] == 'net.minecraft.world.entity.Entity'
                assert resolve(named, row['class_name'], 'getInterpolation') == 'net.minecraft.world.entity.LivingEntity'
                assert resolve(named, row['class_name'], 'moveOrInterpolateTo') == 'net.minecraft.world.entity.Entity'
        roots = {row['class_name'] for row in types.values()}
        roots.update({'net.minecraft.world.entity.EntityType',
                      'net.minecraft.world.level.EntityGetter',
                      'net.minecraft.client.multiplayer.ClientPacketListener'})
        roots.add('net.minecraft.world.entity.vehicle.Boat' if version == '1.16.1'
                  else 'net.minecraft.world.entity.vehicle.boat.AbstractBoat')
        if version != '1.16.1':
            roots.update({'net.minecraft.world.entity.InterpolationHandler',
                          'net.minecraft.world.entity.InterpolationHandler$InterpolationData'})
        result['versions'][version] = {
            'original_client_jar_sha1': jar_sha, 'client_mappings_sha1': mappings_sha,
            'types': types, 'method_sha256': methods(named, roots),
        }
    sources = [
        'scripts/audit_contact_envelopes.py', 'docs/contact-envelope-audit.md',
        'docs/evidence/contact-capabilities-20261014.json',
        'src/client/entity/contact_capabilities.rs', 'src/client/entity/owned_contacts.rs',
        'src/client/entity/interpolation.rs', 'src/client/entity/packet_model.rs',
        'src/client/entity/contact_tick.rs', 'src/client/entity/contact.rs',
        'src/client/physics/mod.rs', 'src/client/physics/boat.rs',
        'src/client/vehicle/collision.rs', 'src/client/vehicle/tests.rs', 'src/client/control.rs',
        'docs/common-entity-owners.md', 'docs/common-entity-dispatch.md',
        'data/client_api/entity_dimensions_oracle.json.gz',
        'data/entities.json', 'data/java_1_21_11/entity_dimensions.json',
        'src/versions/table.rs', 'scripts/generate_version_tables.py',
    ]
    result['source_sha256'] = {name: digest(ROOT / name) for name in sources}
    check_policy(policy, result)
    output = args.output
    output.write_text(json.dumps(result, indent=2) + '\n')
    print('Verified spatial proof owners for both versions; wrote', output)


if __name__ == '__main__':
    main()
