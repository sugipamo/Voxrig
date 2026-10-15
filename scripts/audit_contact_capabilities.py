#!/usr/bin/env python3
"""Generate private owner contact capabilities from verified original clients.

Rebuild remapped originals from hash-verified Mojang clients and mappings,
then export method owners and bytecode fingerprints (never method bodies).
Explicit side-effect reviews are in docs/contact-capability-audit.md.
Unreviewed species remain unknown even when method owners match.
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
METHODS = ['tick', 'handleEntityEvent', 'animateHurt', 'aiStep', 'pushEntities', 'doPush', 'push', 'isPushable',
           'canBeCollidedWith', 'getCollideBox', 'getCollideAgainstBox', 'canCollideWith', 'getDimensions', 'getDefaultDimensions',
           'getAgeScale', 'isBaby', 'onSyncedDataUpdated', 'lerpTo',
           'onClimbable', 'isAlive', 'isSleeping', 'isEffectiveAi', 'getTeam',
           'getScale', 'getInterpolation', 'moveOrInterpolateTo', 'refreshDimensions']


def digest(path, algorithm='sha256'):
    return hashlib.new(algorithm, path.read_bytes()).hexdigest()


def audit(jar):
    def javap(classes):
        return subprocess.check_output(['javap', '-p', '-classpath', str(jar), *classes], text=True)
    raw = javap(['net.minecraft.world.entity.EntityType'])
    types = dict((n.lower(), c) for c, n in re.findall(r'EntityType<([\w.$]+)> (\w+);', raw))
    types['player'] = 'net.minecraft.client.player.RemotePlayer'
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


# Explicit reviewed lanes only. Owner equality is evidence to inspect, never
# a qualification rule. See docs/contact-capability-audit.md for side effects.
PASSIVE_QUALIFIED = {'chicken', 'rabbit', 'ocelot', 'villager', 'wandering_trader',
                     'mooshroom', 'silverfish', 'giant'}
QUALIFIED = {'player', 'cow', 'pig', 'zombie', 'sheep', 'creeper'} | PASSIVE_QUALIFIED
# Reviewed failures are fingerprinted as well; see the per-type blockers.
PASSIVE_UNKNOWN = {
    'llama': 'AbstractHorse client stand/eating geometry and llama size/strength/passenger hooks are not modeled.',
    'trader_llama': 'Llama/AbstractHorse geometry and trader despawn/mount hooks are not closed.',
    'wolf': 'TamableAnimal getTeam resolves owner/root-owner teams with version-specific precedence; reducer uses own UUID team.',
    'cat': 'TamableAnimal getTeam resolves owner/root-owner teams with version-specific precedence; reducer uses own UUID team.',
    'fox': 'Fox isSleeping reads species flags; reducer only handles LivingEntity sleeping-position metadata.',
    'iron_golem': 'Species doPush target/random branch needs separate side-effect closure and numerical coverage.',
    'horse': 'AbstractHorse tick/mount/baby owners lack a closed species proof after the historical legacy sleeping mismatch.',
    'donkey': 'AbstractHorse standing/eating and chested-horse metadata/mount hooks are not closed.',
    'mule': 'AbstractHorse standing/eating and chested-horse metadata/mount hooks are not closed.',
    'skeleton_horse': 'AbstractHorse geometry plus SkeletonHorse aiStep/water overrides are not closed.',
    'zombie_horse': 'AbstractHorse standing/eating and mount hooks are not closed.',
    'panda': 'Panda rolling/sitting/on-back flags and client tick effects are not modeled.',
    'polar_bear': 'Standing metadata changes dimensions outside the generic pose/baby reducer.',
    'dolphin': 'Water/client movement hooks and version-specific age geometry are not closed.',
    'turtle': 'Species baby dimension override and land/water aiStep are not modeled.',
    'bee': 'Flight/anger/sting tick and metadata hooks are not closed.',
    'endermite': 'Species tick lifetime/particles and client side-effect closure lack numerical coverage.',
    'enderman': 'Species synchronized-data/teleport/target hooks are not closed.',
    'witch': 'Species aiStep/item-use and metadata effects lack closed numerical coverage.',
    'vindicator': 'Raider/illager tick, team and metadata overrides are not closed.',
    'pillager': 'Raider/illager tick, team and metadata overrides are not closed.',
    'evoker': 'Raider/spellcaster tick, team and spell metadata overrides are not closed.',
    'ravager': 'Species push/stun/roar tick effects are not modeled.',
    'drowned': 'Zombie subclass swim/travel/metadata overrides are not closed.',
    'husk': 'Zombie subclass variant geometry/callbacks lack independent numerical coverage.',
    'zombified_piglin': 'Species anger/age geometry and callbacks lack independent numerical coverage.',
    'zombie_villager': 'Conversion and villager-variant metadata callbacks lack independent numerical coverage.',
    'snow_golem': 'Species aiStep/environment side effects lack independent numerical coverage.',
    'skeleton': 'Skeleton-specific conversion/item-use hooks lack full variant numerical coverage in both versions.',
    'stray': 'Skeleton ancestry/conversion hooks lack full variant numerical coverage in both versions.',
    'wither_skeleton': 'Skeleton ancestry/item-use hooks lack full variant numerical coverage in both versions.',
}
IRRELEVANT = {'bat', 'armor_stand'}
LANES = ['walking', 'incoming', 'solid', 'geometry', 'metadata', 'status', 'animation']


def classify(version, name, row, metadata):
    result = dict.fromkeys(LANES, 'Unknown')
    if name in QUALIFIED:
        result.update(walking='Qualified', incoming='Qualified',
                      solid='ProvenIrrelevant', geometry='Qualified',
                      metadata='Qualified', status='BoundedUnknown', animation='BoundedUnknown')
    elif name in IRRELEVANT:
        # Geometry itself is not modeled. Unconditional contact exclusions
        # survive geometry/metadata changes in the declared owner lane.
        result.update(walking='ProvenIrrelevant', incoming='ProvenIrrelevant',
                      solid='ProvenIrrelevant', metadata='ProvenIrrelevant',
                      status='ProvenIrrelevant', animation='ProvenIrrelevant')
    elif name in {'horse', 'spider', 'cave_spider', 'ghast'}:
        # Partial WIP ownership evidence only: bounds of effects NOT proven.
        # This classification authorizes no envelope exemption.
        result['walking'] = 'BoundedUnknown'
    return result


def fingerprints(jar, rows):
    # Bind every method body in every ancestor of the explicit reviewed classes,
    # including constructors, tick wrappers, overloads and synthetic callbacks.
    classes = set()
    todo = {rows[n]['class_name'] for n in QUALIFIED | IRRELEVANT | PASSIVE_UNKNOWN.keys() if n in rows}
    todo.add('net.minecraft.world.entity.EntityType')
    todo.add(rows['boat' if 'boat' in rows else 'oak_boat']['class_name'])
    todo.update({'net.minecraft.world.level.EntityGetter',
                 'net.minecraft.client.player.RemotePlayer',
                 'net.minecraft.world.entity.ItemBasedSteering'})
    result = {}
    while todo:
        cls = todo.pop()
        if cls in classes or cls.startswith('java.'):
            continue
        classes.add(cls)
        raw = subprocess.check_output(['javap', '-c', '-p', '-classpath', str(jar), cls], text=True)
        parent = re.search(r'(?:class|interface) [\w.$]+(?:<[^\n]*?>)? extends ([\w.$]+)', raw)
        if parent:
            todo.add(parent[1])
        bodies = {}
        for block in re.split(r'\n(?=  [^ ])', raw):
            if '    Code:' in block:
                signature = block.split('\n')[0].strip()
                bodies[signature] = hashlib.sha256(block.encode()).hexdigest()
        result[cls] = bodies
    return dict(sorted(result.items()))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--downloads', type=Path, required=True)
    parser.add_argument('--work', type=Path, required=True)
    parser.add_argument('--output', type=Path, default=ROOT / 'docs/evidence/contact-capabilities-20261015.json')
    args = parser.parse_args()
    result = {'schema': 2, 'scope': 'Step D; audited ordinary mob extension; Step B envelope policy unchanged', 'versions': {}}
    rust = ['// Generated by scripts/audit_contact_capabilities.py; private owner policy.',
            'use super::{Capabilities, Classification};', 'use crate::MinecraftVersion;',
            'pub(super) fn lookup(version: MinecraftVersion, name: &str) -> Capabilities {',
            '    match version {']
    for version, (jar_sha, mapping_sha) in CLIENT.items():
        assert digest(args.downloads / f'{version}-client.jar', 'sha1') == jar_sha
        assert digest(args.downloads / f'{version}-client-mappings.txt', 'sha1') == mapping_sha
        # Rebuild from the verified original instead of trusting a supplied named.jar.
        tool = args.downloads / 'SpecialSource-1.11.4-shaded.jar'
        assert digest(tool) == 'e2cab24b1c12400ad73b15972bb21e4273a0dc7081c8b3c136ddfdd824c78518'
        named = args.work / version / 'named.jar'
        named.parent.mkdir(parents=True, exist_ok=True)
        subprocess.check_call(['java', '-jar', str(tool), '--in-jar',
            str(args.downloads / f'{version}-client.jar'), '--out-jar', str(named),
            '--srg-in', str(args.downloads / f'{version}-client-mappings.txt')], stdout=subprocess.DEVNULL)
        rows = audit(named)
        for name in PASSIVE_QUALIFIED:
            row = rows[name]
            if name == 'chicken':
                assert row['aiStep'] == ('net.minecraft.world.entity.animal.Chicken'
                    if version == '1.16.1' else 'net.minecraft.world.entity.animal.chicken.Chicken')
            for method in ['pushEntities', 'doPush', 'push', 'isPushable', 'getDimensions', 'isSleeping']:
                assert row[method] == LIVING, (version, name, method, row[method])
            assert row['getTeam'] == 'net.minecraft.world.entity.Entity', (version, name, 'team')
            if version == '1.16.1':
                assert row['getCollideBox'] == 'net.minecraft.world.entity.Entity'
                assert row['lerpTo'] == LIVING
            else:
                assert row['canBeCollidedWith'] == 'net.minecraft.world.entity.Entity'
                assert row['getDefaultDimensions'] == (
                    'net.minecraft.world.entity.animal.chicken.Chicken' if name == 'chicken' else
                    'net.minecraft.world.entity.animal.cow.AbstractCow' if name == 'mooshroom' else LIVING
                ), (version, name, row['getDefaultDimensions'])
                assert row['getAgeScale'] == LIVING
                assert row['getInterpolation'] == LIVING
                assert row['moveOrInterpolateTo'] == 'net.minecraft.world.entity.Entity'
        metadata_path = ROOT / 'data/client_api' / f'entity_data-{version}.json'
        metadata = json.loads(metadata_path.read_text())['entities']
        for name, row in rows.items():
            row['classifications'] = classify(version, name, row, metadata.get('minecraft:' + name, []))
            row['audit'] = 'reviewed-owner-lane' if name in QUALIFIED | IRRELEVANT else 'unqualified'
            if name in PASSIVE_UNKNOWN:
                row['qualification_blocker'] = PASSIVE_UNKNOWN[name]
            elif name in PASSIVE_QUALIFIED:
                row['qualification_review'] = 'docs/contact-capability-audit.md#step-d-ordinary-mob-review'
        result['versions'][version] = dict(original_client_jar_sha1=jar_sha,
            client_mappings_sha1=mapping_sha, entity_data_sha256=digest(metadata_path),
            method_sha256=fingerprints(args.work / version / 'named.jar', rows), types=rows)
        variant = 'Java' + version.replace('.', '_')
        rust.append(f'        MinecraftVersion::{variant} => match name {{')
        groups = {}
        for name, row in rows.items():
            values = tuple(row['classifications'][lane] for lane in LANES)
            if set(values) != {'Unknown'}:
                groups.setdefault(values, []).append(name)
        for values, names in sorted(groups.items()):
            for i, name in enumerate(sorted(names)):
                rust.append(f'            {"" if i == 0 else "| "}"minecraft:{name}"')
            rust.append('                => Capabilities {')
            for lane, value in zip(LANES, values):
                rust.append(f'                    {lane}: Classification::{value},')
            rust.append('                },')
        rust.extend(['            _ => Capabilities::UNKNOWN,', '        },'])
    rust.extend(['    }', '}', ''])
    output = ROOT / 'src/client/entity/contact_capabilities_generated.rs'
    output.write_text('\n'.join(rust))
    subprocess.check_call(['rustfmt', '--edition', '2024', str(output)])
    result['generator_sha256'] = digest(Path(__file__))
    result['audit_sha256'] = digest(ROOT / 'docs/contact-capability-audit.md')
    result['numerical_oracle_sha256'] = digest(ROOT / 'data/client_api/entity_owner_oracle.json.gz')
    result['numerical_runner_sha256'] = digest(ROOT / 'scripts/run_entity_owner_oracle.py')
    result['packet_audit_sha256'] = digest(ROOT / 'data/client_api/contact_packet_audit.json')
    result['generated_sha256'] = digest(output)
    args.output.write_text(json.dumps(result, indent=2) + '\n')


if __name__ == '__main__':
    main()
