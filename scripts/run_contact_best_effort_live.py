#!/usr/bin/env python3
"""Best-effort contact movement on isolated, SHA-1 verified official servers.

Uses the existing RCON oracle and original-byte trace proxy. Native 1.16.1
uses explicit PlayerControl opt-in; 1.21.11 uses body::Move acceptance.
"""
import argparse
import hashlib
import re
from pathlib import Path
from run_climbing_control import run
from run_common_native import until


def check(version, command, request, trace, report):
    modern = version == '1.21.11'
    report['sdk'] = dict(binary_sha256=hashlib.sha256(BINARY.read_bytes()).hexdigest())
    command('difficulty normal')
    command('time set midnight')
    command('gamerule ' + ('minecraft:mob_drops' if modern else 'doMobLoot') + ' true')
    command('fill -8 64 -8 16 64 32 minecraft:stone')

    def names():
        return [e['motion']['entity']['type_name'] for e in request('entities')['entities']]

    def position():
        raw = command('data get entity ClimbingProbe Pos')
        return [float(v.strip().rstrip('df')) for v in re.search(r'\[([^]]+)\]', raw)[1].split(',')]

    def prepare():
        command('tp ClimbingProbe .5 65 .5 0 0')
        assert 'error' not in request('prepare', position=[.5, 65., .5])

    keys = dict(forward=1, strafe=0, jump=False, sneak=False, sprint=False, yaw=0., pitch=0.)

    def move(name, correct=False, diagnostic=True, during=None):
        prepare()
        before = position()
        boundary = trace.mark()
        if modern:
            previous = request('control_record')
            previous_owner = previous['session_id'] if previous else None
            identity = request('submit', kind='move', controls=keys, ticks=30 if correct or during else 10)
            assert 'error' not in identity, identity
            record = until(lambda: (r if r['session_id'] != previous_owner
                                    and r.get('dispatched_ticks', 0) >= 3 else None)
                           if (r := request('control_record')) else None, 6)
            owner = record['session_id']
            if during:
                assert request('contact_diagnostic', session_id=owner) is None
                during()
            if correct:
                command('tp ClimbingProbe 4.5 65 .5 0 0')
            result = until(lambda: (s if s['status'] != 'pending' else None) if (s := request('status', id=identity)) else None, 12)
            assert result['status'] == ('corrected' if correct else 'accepted'), result
            if correct:
                assert 'Pose' in result['correction'], result
            assert request('release') == 'Released'
        else:
            record = request('best_effort_start')
            assert 'error' not in record, record
            owner = record['session_id']
            assert 'error' not in request('control_set', controls=keys)
            start = record['dispatched_ticks']
            if during:
                assert request('contact_diagnostic', session_id=owner) is None
                during()
            record = until(lambda: (r if r['dispatched_ticks'] >= start + 10 else None) if (r := request('control_record')) else None, 8)
            assert record['status']['status'] == 'running', record
            after_move = position()
            if correct:
                previous = record['corrections']
                command('tp ClimbingProbe 4.5 65 .5 0 0')
                record = until(lambda: (r if r['corrections'] > previous else None) if (r := request('control_record')) else None, 8)
                assert record['status']['status'] == 'running' and record['session_id'] == owner, record
            result = request('control_stop')
            assert 'error' not in result, result
        cause = request('contact_diagnostic', session_id=owner)
        assert 'error' not in (cause or {}), cause
        if diagnostic:
            assert cause, (name, record)
        after = position() if modern or correct else after_move
        if not correct:
            assert after[2] > before[2] + .5, (before, after)
        frames = trace.since(boundary)
        # RCON displacement and the controller's complete-write counter provide
        # separate evidence in addition to the full trace retained here.
        assert record['dispatched_ticks'] >= 3, record
        report['checks'].append(dict(name=name, owner=owner, record=record, result=result,
                                     diagnostic=cause, native_before=before, native_after=after,
                                     frames=frames))
        request('wait', ms=100)

    for name in ['fox', 'skeleton', 'horse']:
        command(f'summon minecraft:{name} 1.5 65 1.5 {{Tags:["BestEffort"],NoAI:1b,NoGravity:1b,Invulnerable:1b}}')
        until(lambda: 'minecraft:' + name in names(), 10)
        prepare()
        strict = request('control_start')
        assert 'error' in strict, strict
        report['checks'].append(dict(name='strict_near_' + name + '_remains_available', refusal=strict))
        move('near_' + name)
        if name == 'horse':
            move('near_horse_server_correction', correct=True)
        command('kill @e[tag=BestEffort]')
        until(lambda: 'minecraft:' + name not in names(), 10)

    # Real death, removal and item-stack spawn packets, rather than synthetic items.
    command('summon minecraft:cow 2.5 65 1.5 {Tags:["BestEffortKill"],NoAI:1b}')
    until(lambda: 'minecraft:cow' in names(), 10)
    command('kill @e[tag=BestEffortKill]')
    until(lambda: 'minecraft:item' in names() and 'minecraft:cow' not in names(), 10)
    move('drops_after_kill', diagnostic=False)
    command('kill @e[type=minecraft:item]')
    until(lambda: 'minecraft:item' not in names(), 10)

    def add_near_contact():
        command('summon minecraft:fox 1.5 65 1.5 {Tags:["BestEffortChurn"],NoAI:1b,NoGravity:1b}')
        until(lambda: 'minecraft:fox' in names(), 10)

    move('near_spawn_during_same_owner', during=add_near_contact)
    command('kill @e[tag=BestEffortChurn]')
    until(lambda: 'minecraft:fox' not in names(), 10)
    command('kill @e[type=minecraft:item]')
    until(lambda: 'minecraft:item' not in names(), 10)

    command('summon minecraft:pig 1.5 65 1.5 {Tags:["BestEffortPassenger"],NoAI:1b,NoGravity:1b,Passengers:[{id:"minecraft:zombie",IsBaby:1b,NoAI:1b,NoGravity:1b}]}')
    until(lambda: all('minecraft:' + n in names() for n in ['pig', 'zombie']), 10)
    move('near_living_passenger')
    trace.expect_disconnect()
    request('disconnect')


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--accept-eula', action='store_true', required=True)
    p.add_argument('--binary', type=Path, required=True)
    p.add_argument('--jars', type=Path, required=True)
    p.add_argument('--version', choices=['1.16.1', '1.21.11'], required=True)
    args = p.parse_args()
    BINARY = args.binary.resolve()
    run(args.version, BINARY, args.jars.resolve(), check=check)
