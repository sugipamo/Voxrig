#!/usr/bin/env python3
"""Qualify the recorded half-adder hit geometry on both official servers."""
import argparse
import math
from pathlib import Path
import time

from run_climbing_control import REPO, run


def check(version, command, request, trace, report):
    report['synthetic_fixture_not_historical_reproduction'] = True
    # Translation of Golemkit main's recorded pose/support; the standing platform
    # is one block higher than its recorded world. No source algorithms replaced.
    position = [0.19061256589492, 65., 0.499862279722396]
    support = [2, 66, 4]
    command('setblock 2 66 4 minecraft:stone')
    command('setblock 1 66 4 minecraft:air')
    command('clear ClimbingProbe')
    command('replaceitem entity ClimbingProbe weapon.mainhand minecraft:glass 2' if version == '1.16.1'
            else 'item replace entity ClimbingProbe weapon.mainhand with minecraft:glass 2')
    command('tp ClimbingProbe ' + ' '.join(map(str, position)))
    request('prepare', position=position)
    deadline = time.monotonic() + 5
    while True:
        before = request('player')
        slot = before['inventory']['slots'][36]
        if slot and slot['value']['kind'] == 'item' and slot['value']['item']['count'] == 2:
            break
        if time.monotonic() >= deadline:
            raise RuntimeError('fixture glass not freshly received')
        time.sleep(0.02)
    eye = [position[0], position[1] + 1.62, position[2]]
    hit = [2., 66.5, 4.5]
    center = [2.5, 66.5, 4.5]
    assert math.dist(eye, hit) < 4.5 < math.dist(eye, center)
    geometry = request('placement_check', support=support, face='West')
    if not geometry['reachable'] or geometry['player_intersects_target_cell'] or geometry['blocking_entities']:
        raise RuntimeError('recorded hit geometry not admitted by the precheck')
    boundary = trace.mark()
    receipt = request('use_on_block', support=support, face='West', cursor=[0., .5, .5])
    deadline = time.monotonic() + 5
    while 'Test passed' not in command('execute if block 1 66 4 minecraft:glass'):
        if time.monotonic() >= deadline:
            raise RuntimeError('native server did not place the block at the admitted hit')
        time.sleep(0.02)
    native_inventory = command('data get entity ClimbingProbe Inventory')
    deadline = time.monotonic() + 5
    while True:
        after = request('player')
        slot = after['inventory']['slots'][36]
        if slot and slot['value']['kind'] == 'item' and slot['value']['item']['count'] == 1:
            if slot['source']['sequence'] <= before['receive_sequence']:
                raise RuntimeError('placement reused a predecessor inventory receipt')
            break
        if time.monotonic() >= deadline:
            raise RuntimeError('no fresh material consumption after placement')
        time.sleep(.02)
    original_frames = trace.since(boundary)
    placements = [f for f in original_frames if f['phase'] == 'play'
                  and f['direction'] == 'serverbound'
                  and f['packet_id'] == (0x2d if version == '1.16.1' else 0x3f)]
    if len(placements) != 1:
        raise RuntimeError('placement did not send exactly one original interaction')
    report['checks'].append(dict(name='near_face_outside_support_center_actual_placement',
        before=before, geometry=geometry, receipt=receipt, after=after,
        native_inventory=native_inventory, hit_distance=math.dist(eye, hit),
        center_distance=math.dist(eye, center), original_frames=original_frames))
    boundary = trace.mark()
    refusal = request('use_on_block', support=support, face='East', cursor=[1., .5, .5], expect_rejected=True)
    writes = [f for f in trace.since(boundary) if f['phase'] == 'play'
              and f['direction'] == 'serverbound'
              and f['packet_id'] == (0x2d if version == '1.16.1' else 0x3f)]
    if writes:
        raise RuntimeError('out-of-range original cursor emitted a block interaction')
    report['checks'].append(dict(name='far_face_rejected_before_write', refusal=refusal))
    trace.expect_disconnect()
    request('disconnect')


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', type=Path, default=REPO/'target/debug/examples/climbing_control_probe')
    parser.add_argument('--jars', type=Path)
    parser.add_argument('--version', choices=['1.16.1', '1.21.11'])
    args = parser.parse_args()
    for version in ([args.version] if args.version else ['1.16.1', '1.21.11']):
        run(version, args.binary.resolve(), args.jars, check=check)
