#!/usr/bin/env python3
"""Player contact scope on an unmodified official server; RCON and raw packets.

Uses the existing disposable local server runner (same EULA acceptance as the
other live runners). No public bind, plugin, modified server or Actions result.
"""
import argparse
import hashlib
import json
import re
import struct
from pathlib import Path
from run_climbing_control import REPO, run
from run_common_native import until


def check(version, command, request, trace, report):
    modern = version == '1.21.11'
    report['server_view_distance'] = 10
    report['sdk'] = {
        'binary_sha256': hashlib.sha256(BINARY.read_bytes()).hexdigest(),
        'source_sha256': {str(p.relative_to(REPO)): hashlib.sha256(p.read_bytes()).hexdigest()
                          for p in sorted([*(REPO / 'src').rglob('*.rs'), REPO/'Cargo.toml', REPO/'Cargo.lock',
                                          REPO/'examples/climbing_control_probe.rs', Path(__file__).resolve()])},
    }
    command('gamerule ' + ('minecraft:mob_drops' if modern else 'doMobLoot') + ' false')
    command('difficulty normal')
    command('fill -8 64 -8 8 64 12 minecraft:stone')
    command('forceload add 64 0 80 16')
    command('fill 64 64 0 80 64 16 minecraft:stone')
    command('tp ClimbingProbe .5 65 .5 0 0')
    request('prepare', position=[.5, 65., .5])

    def entities():
        return request('entities')['entities']

    def names():
        return [e['motion']['entity']['type_name'] for e in entities()]

    # Spawn and ride on the original server. A dummy marker or client-side
    # fabricated history cannot establish the regression.
    command('summon minecraft:chicken 64.5 65 .5 {Tags:["ScopeCarrier"],NoAI:1b,NoGravity:1b,Invulnerable:1b,Passengers:[{id:"minecraft:zombie",Tags:["ScopeRider"],NoAI:1b,NoGravity:1b,Invulnerable:1b}]}')
    until(lambda: 'minecraft:chicken' in names() and 'minecraft:zombie' in names(), 15)
    relation = command('data get entity @e[tag=ScopeCarrier,limit=1] Passengers')
    assert 'minecraft:zombie' in relation, relation
    for _ in range(270):
        command('summon minecraft:cow 72.5 65 8.5 {Tags:["ScopeFar"],NoAI:1b,NoGravity:1b,Invulnerable:1b}')
    until(lambda: names().count('minecraft:cow') >= 270, 30)
    before = command('data get entity ClimbingProbe Pos')
    mark = trace.mark()
    started = request('mode_start', mode='survival')
    assert started['admitted'], started
    request('keys', controls=dict(forward=1, strafe=0, jump=False, sneak=False, sprint=False, yaw=0., pitch=0.))
    moving = request('ticks', count=8)
    assert moving['status']['status'] == 'running', moving
    assert moving['frame']['position'][2] > 1., moving
    assert not moving['entity_contacts']['bodies'], moving['entity_contacts']
    stopped = request('stop')
    actual = command('data get entity ClimbingProbe Pos')
    values = lambda text: [float(v.strip().rstrip('df')) for v in re.search(r'\[([^]]+)\]', text)[1].split(',')]
    assert values(actual)[2] > values(before)[2] + .3, (before, actual)
    report['checks'].append(dict(name='movement_with_actual_far_ridden_mob_and_270_far_cows', relation=relation,
        received_entities=len(entities()), start=started, record=moving, native_before=before, native_after=actual,
        frames=trace.since(mark)))

    # Establish an independent received neutral pose after the native stop tail.
    # ControlRecord freezes the last controlled frame; native idle maintenance
    # can settle one further retained step before this explicit receipt fence.
    neutral = [.5, 65., .5]
    command('tp ClimbingProbe .5 65 .5 0 0')
    request('prepare', position=neutral)
    request('wait', ms=200)
    assert max(abs(a-b) for a,b in zip(values(command('data get entity ClimbingProbe Pos')), neutral)) < 1e-7

    # A new adjacent group must refuse before a new controlled movement frame. Numeric ID
    # reuse and relation changes also have unit coverage; this uses real packets.
    pos = neutral
    command('summon minecraft:chicken ' + ' '.join(map(str, [pos[0] + 1., pos[1], pos[2]])) + ' {Tags:["ScopeNear"],NoAI:1b,NoGravity:1b,Invulnerable:1b,Passengers:[{id:"minecraft:zombie",Tags:["ScopeNear"],NoAI:1b,NoGravity:1b,Invulnerable:1b}]}')
    until(lambda: names().count('minecraft:chicken') >= 2, 15)
    mark = trace.mark()
    failed = request('mode_start', mode='survival')
    assert not failed['admitted'] and failed['kind'] == 'Unsupported', failed
    assert 'passenger' in failed['error'], failed
    refusal = failed['movement_refusal']
    assert refusal['Scoped']['geometry']['entity_position'] is not None, refusal
    frames = trace.since(mark)
    movements = {0x12, 0x13, 0x14} if not modern else {0x1d, 0x1e, 0x1f, 0x20}
    assert failed['record']['session_id'] == stopped['session_id'], failed
    assert failed['record']['dispatched_ticks'] == stopped['dispatched_ticks'], failed
    # Native idle maintenance can echo the retained pose after a control stop.
    # It must not advance the rejected operation or invent a new position.
    idle = []
    for f in frames:
        if f['direction'] != 'serverbound' or f['phase'] != 'play' or f['packet_id'] not in movements:
            continue
        if not modern and f['packet_id'] in (0x12, 0x13):
            position = struct.unpack('>ddd', bytes.fromhex(f['body_hex'])[:24])
            assert max(abs(a-b) for a,b in zip(position, neutral)) < 1e-9, (position, neutral)
            idle.append(f)
        else:
            assert not modern, 'unexpected modern controlled movement after rejection'
    failed['retained_pose_echoes'] = idle
    report['checks'].append(dict(name='adjacent_rider_group_refused_before_movement', refusal=failed, frames=frames))

    command('tp @e[tag=ScopeNear] 10000 65 10000')
    def near_removed():
        received = names()
        return received.count('minecraft:chicken') == 1 and received.count('minecraft:zombie') == 1
    until(near_removed, 15)
    restarted = request('mode_start', mode='survival')
    assert restarted['admitted'], restarted
    again = request('ticks', count=3)
    assert again['status']['status'] == 'running', again
    request('stop')
    report['checks'].append(dict(name='new_owner_after_cause_removed_on_same_connection', start=restarted, record=again))
    trace.expect_disconnect()
    request('disconnect')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--jars', type=Path, required=True)
    parser.add_argument('--version', choices=['1.16.1', '1.21.11'], default='1.16.1')
    args = parser.parse_args()
    BINARY = args.binary.resolve()
    run(args.version, BINARY, args.jars.resolve(), check=check,
        server_properties={'view-distance': 10, 'simulation-distance': 10})
