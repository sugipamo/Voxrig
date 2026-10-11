#!/usr/bin/env python3
"""Qualify frozen entity deltas against original frames on official servers."""
import argparse
import copy
import functools
import hashlib
import json
from pathlib import Path
import struct
import subprocess

from run_climbing_control import REPO, run
from run_common_native import NativeSocialReader, PacketTraceProxy, until
from run_entity_pose_live import check as peer_pose_check


def metadata(frame, version):
    r = NativeSocialReader(frame, version)
    entity, entries = r.integer(), []
    old = r.old
    while True:
        index = r.take(1)[0]
        if index == 255:
            r.end()
            return entity, entries, True
        kind = r.integer()
        if kind == 0:
            value = {'Byte': int.from_bytes(r.take(1), 'big', signed=True)}
        elif kind == 1:
            value = {'Int': r.integer()}
        elif not old and kind == 2:
            number = 0
            for shift in range(0, 70, 7):
                byte = r.take(1)[0]
                number |= (byte & 127) << shift
                if byte < 128:
                    break
            value = {'Long': number - (1 << 64) if number & (1 << 63) else number}
        elif kind == (2 if old else 3):
            value = {'Float': struct.unpack('>f', r.take(4))[0]}
        elif kind == (3 if old else 4):
            value = {'String': r.string()}
        elif kind == (7 if old else 8):
            value = {'Bool': r.boolean()}
        elif kind == (18 if old else 20):
            value = {'Pose': r.integer()}
        else:
            value = {'Other': kind}
            if kind == (4 if old else 5):
                r.text()
            elif kind == (5 if old else 6):
                if r.boolean():
                    r.text()
            elif kind == (8 if old else 9):
                r.take(12)
            elif kind == (9 if old else 10):
                r.take(8)
            elif kind == (10 if old else 11):
                if r.boolean():
                    r.take(8)
            elif kind == (12 if old else 13):
                if r.boolean():
                    r.take(16)
            elif kind in ((11, 13, 17) if old else (12, 14, 15, 19, 21, 22, 23, 24, 25, 26, 27, 28, 31, 32, 33, 34, 38)):
                r.integer()
            elif old and kind == 14:
                tag = r.take(1)[0]
                if tag:
                    if old:
                        r.take(int.from_bytes(r.take(2), 'big'))
                    r.nbt(tag)
            elif not old and kind in (16, 17):
                for _ in range(r.integer() if kind == 17 else 1):
                    if r.integer() != 21:
                        return entity, entries, False
                    r.take(4)
            elif not old and kind == 18:
                for _ in range(3):
                    r.integer()
            elif not old and kind == 29:
                if r.boolean():
                    r.string()
                    r.take(8)
            elif not old and kind == 35:
                r.take(12)
            elif not old and kind == 36:
                r.take(16)
            else:
                return entity, entries, False
        entries.append(dict(index=index, value=value))


def equipment(frame, version):
    r = NativeSocialReader(frame, version)
    entity, entries = r.integer(), []
    items = json.loads((REPO / ('data/items.json' if r.old else 'data/java_1_21_11/items.json')).read_text())
    names = {v['id']: 'minecraft:' + v['name'] for v in items}
    version_name = 'Java1_16_1' if r.old else 'Java1_21_11'
    while True:
        slot = r.take(1)[0]
        slot_name = ['MainHand', 'OffHand', 'Feet', 'Legs', 'Chest', 'Head', 'Body', 'Saddle'][slot & 127]
        count = (1 if r.boolean() else 0) if r.old else r.integer()
        if count:
            identifier = r.integer()
            if r.old:
                count = int.from_bytes(r.take(1), 'big', signed=True)
                start = r.cursor
                tag = r.take(1)[0]
                if tag:
                    r.take(int.from_bytes(r.take(2), 'big'))
                    r.nbt(tag)
                data = dict(kind='legacy_nbt', bytes=list(r.raw[start:r.cursor])) if tag else dict(kind='default')
            else:
                added, removed = r.integer(), r.integer()
                assert added == removed == 0, ('nonempty original component patch', added, removed)
                data = dict(kind='default')
            item = dict(id=dict(version=version_name, kind='Item', value=identifier), name=names[identifier], count=count, data=data)
            value = dict(kind='item', item=item)
        else:
            value = dict(kind='empty')
        entries.append(dict(slot=slot_name, value=value))
        if slot & 128 == 0:
            r.end()
            return entity, entries


def attributes(frame, version):
    assert version == '1.21.11'
    r = NativeSocialReader(frame, version)
    entity, values = r.integer(), {}
    for _ in range(r.integer()):
        key, base = r.integer(), struct.unpack('>d', r.take(8))[0]
        modifiers = []
        for _ in range(r.integer()):
            name, amount = r.string(), struct.unpack('>d', r.take(8))[0]
            operation = ['Addition', 'MultiplyBase', 'MultiplyTotal'][r.take(1)[0]]
            modifiers.append(dict(id=name, amount=amount, operation=operation))
        values[key] = dict(base=base, modifiers=modifiers)
    r.end()
    return entity, values


def check(version, command, request, trace, report, binary, sdk):
    report['sdk'] = sdk
    records = {}
    previous = 0

    def drain():
        nonlocal previous
        page = request('entity_history', maximum=7)
        tail = page['latest_cursor']['ordinal']
        while True:
            assert page['gap'] is None, page
            for record in page['records']:
                assert record['ordinal'] > previous
                assert record['receive_sequence'] <= page['receive_sequence']
                records[record['ordinal']] = record
                previous = record['ordinal']
            if page['next_cursor']['ordinal'] >= tail:
                return
            page = request('entity_history', maximum=min(7, tail - page['next_cursor']['ordinal']))

    def frame(sequence, packet):
        frames = [f for f in trace.since(0) if f['direction'] == 'clientbound' and f['phase'] in ('configuration', 'play')]
        original = frames[sequence - 1]
        assert original['phase'] == 'play' and original['packet_id'] == packet
        return original

    def verify_all():
        counts = dict(Metadata=0, Equipment=0, Scale=0)
        for record in records.values():
            for kind in counts:
                if kind not in record['kind']:
                    continue
                delta = record['kind'][kind]
                packet = {'Metadata': 0x44 if version == '1.16.1' else 0x61,
                          'Equipment': 0x47 if version == '1.16.1' else 0x64,
                          'Scale': 0x81}[kind]
                original = frame(record['receive_sequence'], packet)
                if kind == 'Scale':
                    target, fields = attributes(original, version)
                    assert delta['attribute'] == {'Retained': {'value': fields[25]}}
                else:
                    if kind == 'Metadata':
                        target, entries, complete = metadata(original, version)
                    else:
                        target, entries = equipment(original, version)
                        complete = True
                    assert delta['complete'] == complete and delta['entries'] == {'Retained': {'value': entries}}, (delta, entries)
                assert target == delta['native_id']
                if delta['entity'] is not None:
                    assert delta['entity']['native_id'] == target
                    assert delta['entity']['session'] == record['session']
                    assert delta['entity']['spawn_sequence'] < record['receive_sequence']
                counts[kind] += 1
        return counts

    peer_pose_check(version, command, request, trace, report, binary, sdk, disconnect_observer=False)
    drain()
    peer_id = next(c['observation']['entity']['id'] for c in report['checks'] if c['name'] == 'received_crouching')
    poses = [(r['receive_sequence'], entry['value']['Pose']) for r in records.values()
             if 'Metadata' in r['kind'] and r['kind']['Metadata']['entity'] == peer_id
             for entry in r['kind']['Metadata']['entries']['Retained']['value'] if 'Pose' in entry['value']]
    assert any(value == 5 for _, value in poses) and any(value == 0 for _, value in poses)
    report['checks'].append(dict(name='paged_peer_pose_deltas_after_removal',identity=peer_id,poses=poses))

    command('summon minecraft:cow 0.5 65 5.5 {Tags:["DataHistoryCow"],NoAI:1b,NoGravity:1b,Invulnerable:1b,Age:0}')
    def cow():
        return next((e for e in request('entities')['entities'] if e['motion']['entity']['type_name'] == 'minecraft:cow'), None)
    original_cow = until(cow, 10)
    identity = original_cow['motion']['entity']['id']
    baby_index = 15 if version == '1.16.1' else 16
    for baby in [True, False, True, False]:
        command('data modify entity @e[tag=DataHistoryCow,limit=1] Age set value ' + ('-24000' if baby else '0'))
        until(lambda: cow()['metadata'].get(str(baby_index), {}).get('value') == {'Bool': baby}, 10)
        drain()
        report['checks'].append(dict(name='cow_baby_delta_' + str(baby) + '_' + str(len(records)), value=baby, identity=identity))
    for slot, name, count in [('weapon.mainhand', 'stone', 3), ('weapon.offhand', 'diamond', 2), ('weapon.mainhand', 'air', 1)]:
        if version == '1.16.1':
            command(f'replaceitem entity @e[tag=DataHistoryCow,limit=1] {slot} minecraft:{name} {count}')
        else:
            command(f'item replace entity @e[tag=DataHistoryCow,limit=1] {slot} with minecraft:{name} {count}')
        key = 'MainHand' if slot == 'weapon.mainhand' else 'OffHand'
        def established():
            value = cow()['equipment'].get(key, {}).get('value', {})
            return value.get('kind') == 'empty' if name == 'air' else value.get('item', {}).get('name') == 'minecraft:' + name and value['item']['count'] == count
        until(established, 10)
        drain()
        report['checks'].append(dict(name='equipment_' + key + '_' + name,identity=identity,count=count))
    if version == '1.21.11':
        for scale in [0.5, 2., 1.]:
            command('attribute @e[tag=DataHistoryCow,limit=1] minecraft:scale base set ' + str(scale))
            until(lambda: cow()['scale_attribute'] and cow()['scale_attribute']['value']['base'] == scale, 10)
            drain()
            report['checks'].append(dict(name='received_scale_' + str(scale),identity=identity))
    command('kill @e[tag=DataHistoryCow]')
    until(lambda: cow() is None, 10)
    drain()
    frozen = copy.deepcopy(records)
    assert any('Removed' in r['kind'] and r['kind']['Removed']['entity'] == identity for r in records.values())
    counts = verify_all()
    assert counts['Metadata'] > 4 and counts['Equipment'] >= 3
    assert counts['Scale'] >= 3 if version == '1.21.11' else counts['Scale'] == 0
    report['checks'].append(dict(name='all_retained_deltas_equal_original_packet_values', counts=counts, record_count=len(records)))
    repeated = request('entity_history', reset=True)
    assert repeated['gap'] is None and not repeated['has_more']
    by_ordinal = {r['ordinal']: r for r in repeated['records']}
    assert all(by_ordinal[n] == value for n, value in frozen.items())
    report['checks'].append(dict(name='frozen_deltas_after_removal_and_repeat_read', unchanged=len(frozen)))
    before = request('player')['session']
    command('gamemode survival ClimbingProbe')
    command('kill ClimbingProbe')
    until(lambda: request('player')['health']['value']['health'] <= 0, 10)
    request('respawn')
    until(lambda: request('player')['session']['world_generation'] != before['world_generation'], 10)
    after = request('entity_history', reset=True)
    assert any('WorldChanged' in r['kind'] for r in after['records'])
    by_ordinal = {r['ordinal']: r for r in after['records']}
    assert all(by_ordinal[n] == value for n, value in frozen.items())
    report['checks'].append(dict(name='old_world_deltas_survive_actual_respawn',before=before,after=after['session']))
    trace.expect_disconnect()
    closed = request('revoke_entity_history')
    assert closed['revoked'] and closed['history']['gap'] is None
    by_ordinal = {r['ordinal']: r for r in closed['history']['records']}
    assert all(by_ordinal[n] == value for n, value in frozen.items())
    report['checks'].append(dict(name='frozen_deltas_readable_after_revocation',retained=len(by_ordinal)))


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--accept-eula', action='store_true', required=True)
    p.add_argument('--binary', type=Path, required=True)
    p.add_argument('--jars', type=Path, default=REPO / '.local/climbing/downloads')
    p.add_argument('--version', choices=['1.16.1', '1.21.11'], action='append')
    p.add_argument('--compiled-sdk-revision', required=True)
    args = p.parse_args()
    revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True, cwd=REPO).strip()
    compiled = subprocess.check_output(['git', 'rev-parse', args.compiled_sdk_revision], text=True, cwd=REPO).strip()
    paths = ['src', 'examples', 'Cargo.toml', 'Cargo.lock', 'data']
    assert not subprocess.check_output(['git', 'diff', compiled, 'HEAD', '--', *paths], cwd=REPO)
    assert not subprocess.check_output(['git', 'diff', 'HEAD', '--', *paths], cwd=REPO)
    artifacts = ['Cargo.lock', 'scripts/run_entity_data_history_live.py', 'scripts/run_entity_pose_live.py', 'scripts/run_climbing_control.py', 'scripts/run_common_native.py', 'examples/climbing_control_probe.rs']
    sdk = dict(source_revision=revision, binary_build_revision=compiled,
               binary_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest(),
               artifacts={f: hashlib.sha256((REPO / f).read_bytes()).hexdigest() for f in artifacts})
    for version in args.version or ['1.16.1', '1.21.11']:
        run(version, args.binary.resolve(), args.jars.resolve(), check=functools.partial(check, binary=args.binary.resolve(), sdk=sdk),
            trace_factory=functools.partial(PacketTraceProxy, body_capture_filter=lambda frame: frame['direction'] == 'clientbound' and frame['phase'] == 'play' and frame['packet_id'] in (0x44, 0x47, 0x61, 0x64, 0x81) and frame['body_length'] <= 65536))


if __name__ == '__main__':
    main()
