#!/usr/bin/env python3
"""Qualify explicit entity schedules and once-only replay on official servers.

Native tick allocation is an explicit fixture input. A connection, receive
ordinal, RCON position or elapsed time does not establish a client model clock.
Original packet-handler/tick-order qualification is the independent oracle.
"""
import argparse
import functools
import gzip
import hashlib
import json
from pathlib import Path
import struct
import subprocess
from run_climbing_control import REPO, run
from run_common_native import NativeSocialReader, until
from run_entity_pose_live import check as peer_pose_check

GUARDS = dict(local_authoritative=False, ticking=True)


def operation_from_wire(frame, version, ident):
    """Independent bounded original-frame decoding, including every component."""
    modern = version == '1.21.11'
    r = NativeSocialReader(frame, version)
    assert r.integer() == ident
    packet = frame['packet_id']
    angles = lambda: [v*360/256 for v in struct.unpack('>bb', r.take(2))]
    if packet in ((0x33, 0x34) if modern else (0x28, 0x29)):
        value = dict(delta=list(struct.unpack('>hhh', r.take(6))),
                     rotation=angles() if packet == (0x34 if modern else 0x29) else None,
                     ground=r.boolean())
        operation = {'Relative': value}
    elif packet == (0x36 if modern else 0x2a):
        operation = {'Rotation': dict(rotation=angles(), ground=r.boolean())}
    elif packet == (0x23 if modern else 0x56):
        position = list(struct.unpack('>ddd', r.take(24)))
        velocity = list(struct.unpack('>ddd', r.take(24))) if modern else None
        rotation = list(struct.unpack('>ff', r.take(8))) if modern else angles()
        operation = {'Absolute': dict(position=position, velocity=velocity, rotation=rotation,
                                      ground=r.boolean(), reset_base=True)}
    elif packet == (0x51 if modern else 0x3b):
        operation = {'HeadYaw': struct.unpack('>b', r.take(1))[0]*360/256}
    elif packet == (0x63 if modern else 0x46):
        if modern:
            first = r.take(1)[0]
            if first == 0:
                velocity = [0., 0., 0.]
            else:
                packed = (int.from_bytes(r.take(1), 'big') << 8) | first
                packed |= int.from_bytes(r.take(4), 'big') << 16
                scale = first & 3
                if first & 4:
                    scale |= (r.integer() & 0xffffffff) << 2
                velocity = [(min((packed >> (3+axis*15)) & 32767, 32766)*2/32766-1)*scale
                            for axis in range(3)]
        else:
            velocity = [v/8000 for v in struct.unpack('>hhh', r.take(6))]
        operation = {'Velocity': velocity}
    else:
        raise AssertionError(('unqualified original operation', packet))
    r.end()
    return operation


def check(version, command, request, trace, report, binary, sdk):
    report['sdk'] = sdk
    peer_pose_check(version, command, request, trace, report, binary, sdk,
                    disconnect_observer=False)
    modern = version == '1.21.11'
    native = json.loads(gzip.decompress(
        (REPO/'data/client_api/entity_tick_oracle.json.gz').read_bytes()))

    def pose_equal(model, expected):
        assert model['pose_bits']['position'] == [hex(int(v, 16))[2:] for v in expected['position_bits']]
        assert model['pose_bits']['rotation'] == expected['rotation_bits']

    for row in native['versions'][version]['results']:
        if row['seed'] not in (0, 1, 3, 9):
            continue
        assert request('tick_order_new') == {'created': True}
        bodies, active, local = {}, {}, {}
        ordinal, frozen = 0, False

        def action(a):
            nonlocal ordinal, frozen
            op, ident = a['op'], a['native_id']
            if op in ('spawn', 'spawn_player'):
                key = a['key']
                b = dict(key=key, native_id=ident, player=op == 'spawn_player', removed=False,
                         passenger_of=None, passengers=[], legacy_ticking_chunk=True,
                         legacy_in_chunk=True, player_passengers=0)
                assert request('tick_order_admit', body=b) == {'Ok': None}
                assert request('packet_model_new', key=key, pose=a['pose']) == {'Ok': None}
                bodies[key], active[ident], local[key] = b, key, False
                return
            key = active[ident]
            b = bodies[key]
            if op == 'remove_membership':
                # Legacy tick iteration already removes marked roots from its
                # live map. An external later removal then returns false.
                expected = not (not modern and b['removed'])
                found = request('tick_order_remove', native_id=ident)
                assert found == {'Ok': expected}, (a, found)
                del active[ident]
            elif op == 'local':
                local[key] = a['value']
            elif op == 'cancel':
                assert request('packet_model_cancel', key=key)['result'] == {'Ok': True}
            elif op in ('freeze', 'unfreeze'):
                frozen = op == 'freeze'
            elif op in ('removed', 'unticking', 'ticking', 'passenger', 'unride'):
                if op == 'removed':
                    b['removed'] = True
                elif op in ('unticking', 'ticking'):
                    b['legacy_ticking_chunk'] = op == 'ticking'
                else:
                    parent = active[a['vehicle']] if op == 'passenger' else b['passenger_of']
                    if op == 'passenger':
                        b['passenger_of'] = parent
                        bodies[parent]['passengers'].append(key)
                    else:
                        b['passenger_of'] = None
                        bodies[parent]['passengers'].remove(key)
                        assert request('packet_model_pose', key=key, pose=a['model_pose_after'])['result'] == {'Ok': True}
                    assert request('tick_order_update', body=bodies[parent]) == {'Ok': None}
                assert request('tick_order_update', body=b) == {'Ok': None}
            else:
                rotation = lambda r: [v*360/256 for v in r]
                if op == 'relative':
                    packet = {'Relative': dict(delta=a['delta'], ground=a['ground'],
                                              rotation=rotation(a['rotation']) if 'rotation' in a else None)}
                elif op == 'rotation':
                    packet = {'Rotation': dict(rotation=rotation(a['rotation']), ground=a['ground'])}
                else:
                    assert op == 'absolute', op
                    value = dict(position=a['position'], rotation=a['rotation'], ground=a['ground'])
                    if modern:
                        value['velocity'] = a['velocity']
                    packet = {('ModernPositionSync' if modern else 'LegacyTeleport'): value}
                ordinal += 1
                args = dict(key=key, ordinal=ordinal, packet=packet,
                            guards=dict(local_authoritative=local[key], ticking=True))
                applied = request('packet_model_apply', **args)
                duplicate = request('packet_model_apply', **args)
                assert applied['result'] == {'Ok': True} and duplicate['result'] == {'Ok': False}
                assert applied['model'] == duplicate['model']

        callbacks = 0
        for step in row['steps']:
            for a in step['actions']:
                action(a)
            assert request('tick_order_begin', frozen=frozen) == {'Ok': None}
            for i, tick in enumerate(step['ticks']):
                assert request('tick_order_next') == {'Ok': dict(
                    key=tick['key'], native_id=tick['native_id'],
                    dispatch='Ride' if tick['dispatch'] == 'ride' else 'Tick')}
                if tick['dispatch'] != 'ride':
                    pose_equal(request('packet_model_frame', key=tick['key']), tick['before'])
                    if tick['dispatch'] != 'player_tick':
                        result = request('packet_model_step', key=tick['key'], collision_free=True if modern else None)
                        assert result['result'] == {'Ok': True}
                        pose_equal(result['model'], tick['after'])
                        assert result['model']['frame']['interpolation']['remaining_steps'] == tick['remaining_steps']
                if i == 0:
                    for a in step['during_tick']:
                        action(a)
                callbacks += 1
            assert request('tick_order_next') == {'Ok': None}
            assert request('tick_order_finish') == {'Ok': None}
            for state in step['states']:
                pose_equal(request('packet_model_frame', key=state['key']), state['pose'])
        report['checks'].append(dict(name='connected_explicit_native_schedule_'+str(row['seed']),
                                     stages=len(row['steps']), callbacks=callbacks))

    def entity():
        return next((e for e in request('entities')['entities']
                     if e['motion']['entity']['type_name'] == 'minecraft:cow'), None)

    def original(sequence):
        frames = [p for p in trace.since(0) if p['direction'] == 'clientbound'
                  and p['phase'] in ('configuration', 'play')]
        return frames[sequence-1]

    command('summon minecraft:cow 2.5 70 6.5 {Tags:["ReplayCow"],NoAI:1b,NoGravity:1b,Invulnerable:1b,PersistenceRequired:1b}')
    e = until(entity, 10)
    ident = e['motion']['entity']['id']
    spawn = original(ident['spawn_sequence'])
    assert spawn['packet_id'] == (0x01 if modern else 0x02)
    r = NativeSocialReader(spawn, version)
    assert r.integer() == ident['native_id']
    r.take(16)
    assert r.integer() == e['motion']['entity']['native_type_id']
    assert list(struct.unpack('>ddd', r.take(24))) == [2.5, 70., 6.5]
    baseline = request('packet_replay_new', guards=GUARDS)
    assert baseline['created'] == {'Ok': None} and 'Ok' in baseline['applied'], baseline
    assert baseline['model']['frame']['entity'] == ident
    duplicate = request('packet_replay_duplicate', guards=GUARDS)
    assert duplicate['applied'] == {'Ok': 0} and duplicate['model'] == baseline['model']
    report['checks'].append(dict(name='original_spawn_once_only_replay', result=baseline,
                                 original_packet=spawn, duplicate=duplicate))

    previous = e['motion']['position']['source']['sequence']
    for index, position in enumerate(([3.5, 70., 6.5], [3.5, 70., 8.5])):
        command('data merge entity @e[tag=ReplayCow,limit=1] {Pos:['+','.join(str(v)+'d' for v in position)+']}')
        def moved():
            e = entity()
            p = e['motion']['position'] if e else None
            return e if p and p['source']['sequence'] > previous and p['value']['position'] == position else None
        e = until(moved, 10)
        page = request('packet_replay_history', guards=GUARDS)
        assert 'Ok' in page['applied'], page
        records = [v for v in page['history']['records'] if 'Motion' in v['kind']
                   and v['kind']['Motion']['entity']['id'] == ident]
        assert records
        packets = []
        for record in records:
            motion = record['kind']['Motion']
            operation = motion['packet']
            assert operation['source'] == dict(kind='received', sequence=record['receive_sequence'])
            frame = original(record['receive_sequence'])
            assert operation['value'] == operation_from_wire(frame, version, ident['native_id'])
            packets.append(frame)
        duplicate = request('packet_replay_duplicate', guards=GUARDS)
        assert duplicate['applied'] == {'Ok': 0} and duplicate['model'] == page['model']
        steps = []
        for _ in range(3):
            step = request('packet_replay_step', collision_free=True if modern else None)
            assert step['result'] == {'Ok': None}
            steps.append(step)
        assert steps[-1]['model']['frame']['model']['interpolation']['remaining_steps'] == 0
        assert steps[-1]['model']['frame']['model']['interpolation']['pose']['position'] == position
        report['checks'].append(dict(name='original_motion_then_explicit_steps_'+str(index),
                                     received=page, original_packets=packets, steps=steps, duplicate=duplicate))
        previous = e['motion']['position']['source']['sequence']
    cancel = request('packet_replay_cancel')
    assert cancel['result'] == {'Ok': None}
    assert cancel['model']['frame']['model']['interpolation']['remaining_steps'] == 0
    command('kill @e[tag=ReplayCow]')
    until(lambda: not entity(), 10)
    dead = request('packet_replay_history', guards=GUARDS)
    assert 'Err' in dead['applied'] and not dead['model']['frame']['valid'], dead
    assert request('packet_replay_step', collision_free=True if modern else None)['result'] == {'Err': 'Invalidated'}
    report['checks'].append(dict(name='removed_lifetime_stops_continuation', result=dead, cancelled=cancel))
    command('summon minecraft:cow 2.5 70 6.5 {Tags:["ReplayCow"],NoAI:1b,NoGravity:1b,Invulnerable:1b,PersistenceRequired:1b}')
    fresh = until(entity, 10)
    assert fresh['motion']['entity']['id'] != ident
    new = request('packet_replay_new', guards=GUARDS)
    assert new['created'] == {'Ok': None} and 'Ok' in new['applied'], new
    trace.expect_disconnect()
    revoked = request('revoke_entity_history')
    assert not revoked['replay']['frame']['valid'], revoked
    report['checks'].append(dict(name='fresh_spawn_and_revocation_fence', recreated=new, revoked=revoked))


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--accept-eula', action='store_true', required=True)
    p.add_argument('--binary', type=Path, required=True)
    p.add_argument('--jars', type=Path, default=REPO/'.local/climbing/downloads')
    p.add_argument('--version', choices=['1.16.1', '1.21.11'], action='append')
    p.add_argument('--compiled-sdk-revision', required=True)
    args = p.parse_args()
    revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=REPO, text=True).strip()
    compiled = subprocess.check_output(['git', 'rev-parse', args.compiled_sdk_revision], cwd=REPO, text=True).strip()
    # The driver may be corrected without changing the compiled SDK. Its
    # actual checkout and hash are recorded separately from the binary build.
    paths = ['src', 'examples', 'Cargo.toml', 'Cargo.lock', 'data']
    assert not subprocess.check_output(['git', 'diff', compiled, 'HEAD', '--', *paths], cwd=REPO)
    assert not subprocess.check_output(['git', 'diff', 'HEAD', '--', *paths], cwd=REPO)
    artifacts = ['Cargo.lock', 'scripts/run_entity_tick_live.py', 'scripts/run_entity_tick_oracle.py',
                 'scripts/entity_tick_oracle/1.16.1/EntityTickOracle.java',
                 'scripts/entity_tick_oracle/1.21.11/EntityTickOracle.java',
                 'scripts/run_entity_pose_live.py', 'scripts/run_climbing_control.py',
                 'scripts/run_common_native.py', 'examples/climbing_control_probe.rs',
                 'data/client_api/entity_tick_oracle.json.gz']
    sdk = dict(source_revision=revision, binary_build_revision=compiled,
               binary_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest(),
               artifacts={f: hashlib.sha256((REPO/f).read_bytes()).hexdigest() for f in artifacts})
    for version in args.version or ['1.16.1', '1.21.11']:
        run(version, args.binary.resolve(), args.jars.resolve(),
            check=functools.partial(check, binary=args.binary.resolve(), sdk=sdk))


if __name__ == '__main__':
    main()
