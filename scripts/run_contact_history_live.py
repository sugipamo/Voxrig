#!/usr/bin/env python3
"""History warnings and uninterrupted owners on isolated official servers.

The original-byte proxy never alters packets. A boat beyond a zombie's tracking
range (native), or a server-only marker rider (modern), supplies a real unresolved
passenger lifetime without deleting SDK data.
Both versions use the explicit best-effort PlayerControl path; the step-1 runner
separately exercises modern body acceptance and bounded correction reporting.
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
    command('gamerule ' + ('minecraft:mob_drops' if modern else 'doMobLoot') + ' false')
    command('fill -8 64 -8 16 64 32 minecraft:stone')
    if not modern:
        command('forceload add 128 0 144 16')
        command('fill 128 64 0 144 64 16 minecraft:stone')
    command('forceload add 64 0 64 16')
    command('fill 64 64 0 64 64 16 minecraft:stone')
    keys = dict(forward=1, strafe=0, jump=False, sneak=False, sprint=False, yaw=0., pitch=0.)

    def names():
        return [e['motion']['entity']['type_name'] for e in request('entities')['entities']]

    def position():
        raw = command('data get entity ClimbingProbe Pos')
        return [float(v.strip().rstrip('df')) for v in re.search(r'\[([^]]+)\]', raw)[1].split(',')]

    def prepare():
        command('tp ClimbingProbe .5 65 .5 0 0')
        assert 'error' not in request('prepare', position=[.5, 65., .5])

    def ticks(owner, count):
        before = request('control_record')['dispatched_ticks']
        record = until(lambda: (r if r['dispatched_ticks'] >= before + count else None)
                       if (r := request('control_record')) else None, 10)
        assert record['session_id'] == owner and record['status']['status'] == 'running', record
        assert record['controls'] == keys, record
        assert record['movement_refusal'] is None, record
        return record

    def warning(owner):
        value = request('contact_diagnostic', session_id=owner)
        assert 'RemoteStateDropped' in value, value
        return value

    def walk(name, during=None, correction=False, strict=True, expected=None):
        prepare()
        refusal = request('control_start') if strict else None
        if strict:
            assert 'error' in refusal, refusal
        start_pos, boundary = position(), trace.mark()
        started = request('best_effort_start')
        assert 'error' not in started, started
        owner = started['session_id']
        assert 'error' not in request('control_set_for', session_id=owner, controls=keys)
        initial = ticks(owner, 3)
        if during:
            during(owner)
        if correction:
            command('tp ClimbingProbe 4.5 65 .5 0 0')
            corrected = until(lambda: (r if r['corrections'] > initial['corrections'] else None)
                              if (r := request('control_record')) else None, 8)
            assert corrected['session_id'] == owner and corrected['status']['status'] == 'running', corrected
        record = ticks(owner, 8)
        diagnostic = warning(owner)
        if expected:
            assert expected in str(diagnostic), diagnostic
        after = position()
        if correction:
            assert abs(after[0] - 4.5) < .1 and after[2] > 1., after
        else:
            assert after[2] > start_pos[2] + .5, (start_pos, after)
        stopped = request('control_stop')
        assert 'error' not in stopped, stopped
        assert request('contact_diagnostic', session_id=owner) == diagnostic
        # A different selected ID must never mutate or read this owner.
        assert 'error' in request('control_set_for', session_id=owner + 1, controls=keys)
        assert 'error' in request('contact_diagnostic', session_id=owner + 1)
        report['checks'].append(dict(name=name, owner=owner, strict_refusal=refusal,
            record=record, warning=diagnostic, native_before=start_pos, native_after=after,
            frames=trace.since(boundary)))

    # Boat tracking range is ten chunks; zombie tracking range is eight.
    # The server sends the boat's passenger IDs while the rider is untracked.
    # Modern advertises a four-chunk view: use a server-only marker passenger
    # nearby instead (a real relationship packet, with no client spawn packet).
    boat = 'oak_boat' if modern else 'boat'
    rider = 'marker' if modern else 'zombie'
    x = 8 if modern else 140
    command(f'summon minecraft:{boat} {x} 65 .5 {{Tags:["HistoryBoat"],NoGravity:1b,Passengers:[{{id:"minecraft:{rider}",Tags:["HistoryRider"],NoAI:1b,NoGravity:1b}}]}}')
    until(lambda: 'minecraft:' + boat in names(), 15)
    assert 'minecraft:' + rider not in names(), names()
    walk('untracked_passenger_relationship', expected='rider')
    walk('missing_relationship_correction_reanchors', correction=True, expected='rider')
    command('kill @e[tag=HistoryBoat]')
    command('kill @e[tag=HistoryRider]')
    until(lambda: not names(), 10)

    command('summon minecraft:pig 6.5 65 1.5 {Tags:["HistoryPig"],NoAI:1b,NoGravity:1b,Passengers:[{id:"minecraft:zombie",Tags:["HistoryRider"],IsBaby:1b,NoAI:1b,NoGravity:1b}]}')
    until(lambda: all('minecraft:' + n in names() for n in ['pig', 'zombie']), 10)
    walk('near_living_passenger')
    command('kill @e[tag=HistoryPig]')
    command('kill @e[tag=HistoryRider]')
    until(lambda: not names(), 10)

    def churn(owner):
        for _ in range(3):
            command('summon minecraft:fox 6.5 65 1.5 {Tags:["HistoryChurn"],NoAI:1b,NoGravity:1b}')
            until(lambda: 'minecraft:fox' in names(), 10)
            ticks(owner, 2)
            warning(owner)
            command('kill @e[tag=HistoryChurn]')
            until(lambda: not names(), 10)
            ticks(owner, 2)

    walk('spawn_despawn_during_same_held_input', strict=False, during=churn)

    def reenter(owner):
        command('tp @e[tag=HistoryCow] 6.5 65 1.5')
        until(lambda: request('contact_diagnostic', session_id=owner), 10)

    far_x = 40.5 if modern else 64.5
    command(f'summon minecraft:cow {far_x} 65 .5 {{Tags:["HistoryCow"],NoAI:1b,NoGravity:1b}}')
    until(lambda: 'minecraft:cow' in names(), 10)
    walk('scope_reselection_after_remote_teleport', strict=False, during=reenter, expected='re-entry')
    command('kill @e[tag=HistoryCow]')
    until(lambda: not names(), 10)
    # Start with a valid contact origin; correction must drop it before refresh.
    walk('intact_remote_origin_correction_reanchors', strict=False, correction=True,
         expected='received pose correction')
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
    run(args.version, BINARY, args.jars.resolve(), check=check,
        server_properties={'view-distance': 4 if args.version == '1.21.11' else 10,
                           'simulation-distance': 4 if args.version == '1.21.11' else 10},
        rcon_timeout=30)
