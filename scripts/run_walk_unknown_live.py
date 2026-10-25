#!/usr/bin/env python3
"""Bounded best-effort walking on SHA-1 checked official servers.

Reuses the existing RCON oracle and original-byte proxy. Unknown terrain is real
not-yet-delivered chunk data: a timing gate delays bytes without editing packets
or deleting SDK state. Also covers partial support, shapes and server corrections.
"""
import argparse
import hashlib
import json
import re
import shutil
import threading
from pathlib import Path

import run_climbing_control as harness
from run_common_native import until


def check(version, command, request, trace, report):
    report['binary_sha256'] = hashlib.sha256(BINARY.read_bytes()).hexdigest()
    keys = dict(forward=1, strafe=0, jump=False, sneak=False, sprint=False, yaw=0., pitch=0.)
    route = [dict(yaw=0., input=dict(forward=1, strafe=0, jump=False))] * 8

    raw_request = request
    def request(action, **arguments):
        for _ in range(8):
            result = raw_request(action, **arguments)
            if action != 'control_record' or not result or 'error' in result:
                return result
            diagnostics = raw_request('walking_diagnostics', session_id=result['session_id'])
            if 'error' in diagnostics:
                # Owner replacement between these separate read-only captures
                # correctly fences the old diagnostic ID; capture the new one.
                continue
            result['terrain_diagnostic'] = diagnostics['terrain']
            result['path'] = diagnostics['path']
            return result
        raise AssertionError('control owner changed during every diagnostic capture')

    def position():
        raw = command('data get entity ClimbingProbe Pos')
        return [float(v.strip().rstrip('df')) for v in re.search(r'\[([^]]+)\]', raw)[1].split(',')]

    def prepare(pos):
        command('tp ClimbingProbe ' + ' '.join(map(str, pos)) + ' 0 0')
        result = request('prepare_pose', position=pos)
        assert 'error' not in result, result

    def complete(identity):
        def poll():
            result = request('status', id=identity)
            assert 'error' not in result, result
            return result if result['status'] != 'pending' else None
        result = until(poll, 12)
        request('wait', ms=80)  # Allow owned cleanup to retire.
        return result

    def move(name, ticks=8, controls=None, expect=('accepted',)):
        mark, before = trace.mark(), position()
        identity = request('submit', kind='move', controls=controls or keys, ticks=ticks)
        assert 'error' not in identity, identity
        result = complete(identity)
        assert result['status'] in expect, (name, result)
        record = request('control_record')
        entry = dict(name=name, result=result, record=record, native_before=before,
                     native_after=position(), frames=trace.since(mark))
        report['checks'].append(entry)
        return entry

    command('fill -8 64 -8 12 64 32 minecraft:stone')
    command('fill -4 65 -4 6 70 30 minecraft:air')
    prepare([.5, 65., .5])

    # Partial support: centre cell is air but the body overlaps the stone edge.
    command('fill 1 64 -2 3 64 2 minecraft:air')
    prepare([1.05, 65., .5])
    edge = move('partial_edge_support', controls=dict(keys, yaw=90.))
    assert edge['native_after'][0] < .8, edge
    command('fill 1 64 -2 3 64 2 minecraft:stone')

    # Native shape path: no exact-rest proof between finite body moves.
    command('fill 0 65 2 0 65 3 minecraft:stone_slab[type=bottom,waterlogged=false]')
    command('setblock 0 65 4 minecraft:stone_stairs[facing=south,half=bottom,shape=straight,waterlogged=false]')
    command('fill 0 65 5 0 65 12 minecraft:stone')
    prepare([.5, 65., .5])
    climbed = move('slabs_stairs', ticks=32)
    assert climbed['native_after'][1] >= 65.5 and climbed['native_after'][2] > 4., climbed
    drop = move('drop_from_known_shapes', ticks=32, controls=dict(keys, yaw=180.))
    assert drop['native_after'][1] < 65.2, drop

    command('fill 3 65 -2 3 65 12 minecraft:oak_fence[east=false,north=true,south=true,west=false,waterlogged=false]')
    prepare([2.5, 65., .5])
    fence = move('walking_beside_fences', ticks=12)
    assert fence['native_after'][2] > 1., fence

    command('fill -4 65 -4 6 70 30 minecraft:air')
    prepare([.5, 65., .5])

    # A real unsupported positional collision shape next to a walkable floor.
    command('setblock 1 65 0 minecraft:moving_piston[facing=north,type=normal]')
    until(lambda: request('observe', blocks=[[1,65,0]])['blocks'][0]['state']['value']['name'] == 'minecraft:moving_piston')
    strict = request('path_start', controls=route)
    assert 'error' in strict, strict
    warned = move('unprovable_shape_body')
    assert warned['record']['terrain_diagnostic'], warned
    assert warned['native_after'][2] > warned['native_before'][2] + .3, warned
    warned['strict_refusal'] = strict

    prepare([.5, 65., .5])
    mark = trace.mark()
    started = request('path_best_effort_start', controls=route)
    assert 'error' not in started, started
    assert 'error' in request('control_set_for', session_id=started['session_id'], controls=keys)
    ended = until(lambda: (r if r['status']['status'] == 'stopped' else None)
                  if (r := request('control_record')) else None, 8)
    assert ended['path'] == dict(total_ticks=8, dispatched_ticks=8), ended
    assert ended['terrain_diagnostic'], ended
    assert position()[2] > .8
    report['checks'].append(dict(name='unprovable_shape_route', record=ended,
                                native_after=position(), frames=trace.since(mark)))

    prepare([.5, 65., .5])
    started = request('path_best_effort_start', controls=route * 2)
    assert 'error' not in started, started
    until(lambda: (r if r['dispatched_ticks'] >= 2 else None)
          if (r := request('control_record')) else None)
    command('tp ClimbingProbe 4.5 65 .5 0 0')
    ended = until(lambda: (r if r['status']['status'] == 'stopped' else None)
                  if (r := request('control_record')) else None, 8)
    assert ended['session_id'] == started['session_id'] and ended['corrections'] >= 1, ended
    assert ended['path']['dispatched_ticks'] == 16, ended
    assert position()[0] > 4.4 and position()[2] > 1., ended
    report['checks'].append(dict(name='finite_route_correction_reanchors_without_replay', record=ended, native_after=position()))

    prepare([.5, 65., .5])
    started = request('best_effort_start')
    owner = started['session_id']
    assert 'error' not in request('control_set_for', session_id=owner, controls=keys)
    until(lambda: (r if r['dispatched_ticks'] >= 3 else None)
          if (r := request('control_record')) else None)
    command('tp ClimbingProbe 4.5 65 .5 0 0')
    corrected = until(lambda: (r if r['corrections'] >= 1 and r['dispatched_ticks'] >= 10 else None)
                      if (r := request('control_record')) else None, 8)
    assert corrected['session_id'] == owner and corrected['status']['status'] == 'running', corrected
    assert corrected['controls'] == keys and position()[0] > 4.4, corrected
    assert position()[2] > .7
    report['checks'].append(dict(name='continuous_correction_reanchors_same_owner', record=corrected, native_after=position()))
    request('control_stop')

    prepare([.5, 65., .5])
    identity = request('submit', kind='move', controls=keys, ticks=16)
    until(lambda: (r if r and r['dispatched_ticks'] >= 2 and r['status']['status'] == 'running' else None)
          if (r := request('control_record')) else None)
    command('tp ClimbingProbe 4.5 65 .5 0 0')
    corrected = complete(identity)
    assert corrected['status'] == 'corrected', corrected
    report['checks'].append(dict(name='body_pose_corrected', result=corrected))
    continued = move('body_after_correction')
    assert continued['native_after'][2] > continued['native_before'][2] + .3, continued
    command('setblock 1 65 0 minecraft:air')

    # A distant column has never reached the client. Delay its original chunk
    # frames; the teleport receipt preceding them still gives a valid anchor.
    command('forceload add 1020 -4 1040 24')
    request('wait', ms=2000)
    command('fill 1020 64 -4 1040 64 24 minecraft:stone')
    seen, release = threading.Event(), threading.Event()
    chunk_packet = 0x21 if version == '1.16.1' else 0x2c
    def delay_chunk(record):
        if record['direction'] == 'clientbound' and record['phase'] == 'play' and record['packet_id'] == chunk_packet:
            seen.set()
            if not release.wait(15):
                raise TimeoutError('original chunk timing gate expired')
    trace.before_forward = delay_chunk
    try:
        prepare([1024.5, 65., .5])
        assert seen.wait(4), 'no original chunk reached gate'
        unknown = request('observe', blocks=[[1024,64,0]])
        assert unknown['blocks'][0]['state'] is None, unknown
        walked = move('unloaded_chunk_body', ticks=5)
        assert walked['record']['terrain_diagnostic'], walked
        assert walked['native_after'][2] > walked['native_before'][2] + .2, walked
        started = request('path_best_effort_start', controls=route)
        assert 'error' not in started, started
        ended = until(lambda: (r if r['status']['status'] == 'stopped' else None)
                      if (r := request('control_record')) else None, 8)
        assert ended['path']['dispatched_ticks'] == 8, ended
        assert ended['terrain_diagnostic'], ended
        report['checks'].append(dict(name='unloaded_chunk_route', record=ended, native_after=position()))
        started = request('best_effort_start')
        owner = started['session_id']
        assert 'error' not in request('control_set_for', session_id=owner, controls=keys)
        paused = until(lambda: (r if r['status']['status'] == 'paused' else None)
                       if (r := request('control_record')) else None, 8)
        assert paused['terrain_diagnostic']['waiting_for_evidence'], paused
        assert paused['terrain_diagnostic']['unproven_ticks'] == 20, paused
        request('wait', ms=300)
        assert request('control_record')['dispatched_ticks'] == paused['dispatched_ticks']
        report['checks'].append(dict(name='no_fresh_evidence_bounded_across_owners', record=paused))
    finally:
        release.set()
        trace.before_forward = lambda record: None
    recovered = until(lambda: (r if r['status']['status'] == 'running' and
                               r['dispatched_ticks'] >= paused['dispatched_ticks'] + 5 else None)
                      if (r := request('control_record')) else None, 10)
    assert recovered['session_id'] == owner, recovered
    assert recovered['terrain_diagnostic']['unproven_ticks'] == 0, recovered
    assert 'error' in request('control_set_for', session_id=owner+1, controls=keys)
    report['checks'].append(dict(name='fresh_chunk_resumes_same_owner', record=recovered, native_after=position()))
    request('control_stop')
    trace.expect_disconnect()
    request('disconnect')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--accept-eula', action='store_true', required=True)
    parser.add_argument('--version', choices=['1.16.1', '1.21.11'], required=True)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--jars', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    BINARY = args.binary.resolve()
    args.output.mkdir(parents=True, exist_ok=True)
    harness.ROOT = args.output.resolve() / 'runs'
    try:
        harness.run(args.version, BINARY, args.jars.resolve(), check=check)
    finally:
        if harness.ROOT.exists():
            for folder in harness.ROOT.iterdir():
                for name in ('report.json', 'packets.jsonl', 'probe.log', 'probe.stderr.log', 'server.log'):
                    if (folder/name).exists():
                        shutil.copyfile(folder/name, args.output/name)
