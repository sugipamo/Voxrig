#!/usr/bin/env python3
"""Generate private owner contact capabilities from verified original clients.

Run run_entity_owner_oracle.py first. This exports method owners, not original
code. The audited wrapper families below have no client movement beyond the
inherited AI-step lane. New/overridden hooks remain unknown by default.
"""
import argparse
import hashlib
import json
import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
CLIENT = {
    '1.16.1': ('c9abbe8ee4fa490751ca70635340b7cf00db83ff', 'ddf517a4f6750f4c15189de4e03246ae1f916cf5'),
    '1.21.11': ('ba2df812c2d12e0219c489c4cd9a5e1f0760f5bd', '031a68bebf55d824f66d6573d8c752f0e1bf232a'),
}
LIVING = 'net.minecraft.world.entity.LivingEntity'
METHODS = ['aiStep', 'pushEntities', 'doPush', 'push', 'isPushable',
           'canBeCollidedWith', 'getDimensions', 'getDefaultDimensions',
           'getAgeScale', 'isBaby', 'onSyncedDataUpdated', 'lerpTo',
           'onClimbable', 'isAlive', 'isSleeping', 'isEffectiveAi']


def digest(path, algorithm='sha256'):
    return hashlib.new(algorithm, path.read_bytes()).hexdigest()


def audit(jar):
    def javap(classes):
        return subprocess.check_output(['javap', '-p', '-classpath', str(jar), *classes], text=True)
    raw = javap(['net.minecraft.world.entity.EntityType'])
    types = dict((n.lower(), c) for c, n in re.findall(r'EntityType<([\w.$]+)> (\w+);', raw))
    cache = {}
    todo = set(types.values())
    while todo:
        batch = sorted(todo)
        todo = set()
        for block in javap(batch).split('Compiled from ')[1:]:
            header = re.search(r'(?:class|interface) ([\w.$]+)(?:<[^\n]*?>)?(?: extends ([\w.$]+))?', block)
            if not header:
                continue
            cls, parent = header.groups()
            cache[cls] = (parent, block)
            if parent and parent not in cache and parent not in batch:
                todo.add(parent)

    def owner(cls, method):
        parent, block = cache.get(cls, (None, ''))
        # push(double,double,double) is distinct from the contact callback.
        argument = r'net\.minecraft\.world\.entity\.Entity\)' if method == 'push' else ''
        if re.search(r' ' + re.escape(method) + r'\(' + argument, block):
            return cls
        return owner(parent, method) if parent else None

    return {n: dict(class_name=c, **{m: owner(c, m) for m in METHODS}) for n, c in types.items()}


def classify(version, name, row, metadata):
    # Metadata ancestry supplements the original EntityType generic class;
    # it is never used as evidence of callback semantics.
    living = any(r[0] == 'LivingEntity' for r in metadata)
    ordinary_ai = row['aiStep'] in {
        LIVING, 'net.minecraft.world.entity.Mob',
        'net.minecraft.world.entity.animal.Animal',
        'net.minecraft.world.entity.monster.Monster',
        'net.minecraft.world.entity.AgableMob', 'net.minecraft.world.entity.AgeableMob',
        'net.minecraft.world.entity.animal.Sheep', 'net.minecraft.world.entity.animal.sheep.Sheep',
    }
    geometry = row['getDimensions'] == LIVING and (
        version == '1.16.1' or (
            row['getDefaultDimensions'] == LIVING and row['getAgeScale'] == LIVING))
    predicates = all(row[m] == LIVING for m in ['onClimbable', 'isAlive', 'isSleeping'])
    motion = row['isEffectiveAi'] == 'net.minecraft.world.entity.Mob'
    solid = row['canBeCollidedWith'] in {None, 'net.minecraft.world.entity.Entity'}
    callback = row['onSyncedDataUpdated'] in {
        LIVING, 'net.minecraft.world.entity.AgableMob', 'net.minecraft.world.entity.AgeableMob'}
    hooks = all(row[m] == LIVING for m in ['pushEntities', 'doPush', 'push', 'isPushable'])
    # Already bytecode/numerically audited non-generic dimensions and wrappers.
    previous = name in {'player', 'cow', 'pig', 'zombie', 'sheep', 'creeper'}
    # Vex.tick wraps the inherited AI-step in noPhysics=true.
    if name == 'vex':
        return 'Unknown'
    if previous or (living and ordinary_ai and motion and solid and geometry and predicates and callback and hooks):
        return 'RemotePlayer' if name == 'player' else 'Inherited'
    if name in {'bat', 'armor_stand', 'item', 'arrow', 'spectral_arrow', 'trident',
                'egg', 'snowball', 'ender_pearl', 'experience_bottle', 'potion',
                'splash_potion', 'lingering_potion'}:
        return 'Irrelevant'
    # A known client interpolation/geometry lane is enough for conservative
    # relevance, even when a contact predicate/doPush override is unqualified.
    if living and ordinary_ai and motion and solid and geometry and callback and row['pushEntities'] == LIVING:
        return 'BoundedUnknown'
    # Horse's inherited lerp/AI-step has only client tail animation around
    # Animal.aiStep; baby/default dimensions never exceed adult type dimensions.
    # Its vehicle-sensitive pushability remains unsupported, not cow semantics.
    if name == 'horse':
        return 'BoundedUnknown'
    return 'Unknown'


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--downloads', type=Path, required=True)
    parser.add_argument('--work', type=Path, required=True)
    args = parser.parse_args()
    result = {'schema': 1, 'versions': {}}
    rust = ['// Generated by scripts/audit_contact_capabilities.py; private owner policy.',
            'use super::Capability;', 'use crate::MinecraftVersion;',
            'pub(super) fn lookup(version: MinecraftVersion, name: &str) -> Capability {',
            '    match version {']
    for version, (jar_sha, mapping_sha) in CLIENT.items():
        assert digest(args.downloads / f'{version}-client.jar', 'sha1') == jar_sha
        assert digest(args.downloads / f'{version}-client-mappings.txt', 'sha1') == mapping_sha
        rows = audit(args.work / version / 'named.jar')
        metadata_path = ROOT / 'data/client_api' / f'entity_data-{version}.json'
        metadata = json.loads(metadata_path.read_text())['entities']
        for name, row in rows.items():
            row['capability'] = classify(version, name, row, metadata.get('minecraft:' + name, []))
        result['versions'][version] = dict(original_client_jar_sha1=jar_sha,
            client_mappings_sha1=mapping_sha, entity_data_sha256=digest(metadata_path), types=rows)
        variant = 'Java' + version.replace('.', '_')
        rust.append(f'        MinecraftVersion::{variant} => match name {{')
        for capability in ['Inherited', 'RemotePlayer', 'Irrelevant', 'BoundedUnknown']:
            names = sorted(n for n, r in rows.items() if r['capability'] == capability)
            for name in names:
                rust.append(f'            "minecraft:{name}" => Capability::{capability},')
        rust.extend(['            _ => Capability::Unknown,', '        },'])
    rust.extend(['    }', '}', ''])
    output = ROOT / 'src/client/entity/contact_capabilities_generated.rs'
    output.write_text('\n'.join(rust))
    result['generator_sha256'] = digest(Path(__file__))
    result['generated_sha256'] = digest(output)
    (ROOT / 'docs/evidence/contact-capabilities-20261014.json').write_text(json.dumps(result, indent=2) + '\n')


if __name__ == '__main__':
    main()
