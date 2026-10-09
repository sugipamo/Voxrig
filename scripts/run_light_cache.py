#!/usr/bin/env python3
"""Observe certified and light-affecting changes on an isolated official 1.16.1 server."""
import argparse
from pathlib import Path
import time

from run_climbing_control import REPO, run
from run_common_native import PacketTraceProxy


POSITIONS = [[x, 65, z] for x in [-1, 0, 16] for z in [-1, 0, 16]] + [
    [2, 64, 2], [2, 65, 2], [2, 0, 2], [2, 15, 2]]
TARGET_INDEX = POSITIONS.index([2, 64, 2])


def light_values(samples):
    return [(s['sky_light'], s['block_light']) for s in samples]


def known(samples):
    return all(s['state'] is not None and s['sky_light'] is not None
               and s['block_light'] is not None for s in samples)


def light_header(frame):
    body = bytes.fromhex(frame['light_header_hex'])
    offset = 0
    coordinates = []
    for _ in range(2):
        value, consumed = PacketTraceProxy.varint(body[offset:])
        coordinates.append(value - (1 << 32) if value >= (1 << 31) else value)
        offset += consumed
    trust_edges = bool(body[offset])
    offset += 1
    masks = []
    for _ in range(4):
        value, consumed = PacketTraceProxy.varint(body[offset:])
        masks.append(value)
        offset += consumed
    return dict(chunk=coordinates, trust_edges=trust_edges,
                sky_mask=masks[0], block_mask=masks[1],
                empty_sky_mask=masks[2], empty_block_mask=masks[3],
                original_frame=frame)


def wait_for(read, predicate, reason, timeout=10):
    deadline = time.monotonic() + timeout
    while True:
        value = read()
        if predicate(value):
            return value
        if time.monotonic() >= deadline:
            raise RuntimeError(reason + ': ' + repr(value))
        time.sleep(.02)


def check(version, command, request, trace, report):
    assert version == '1.16.1'
    report['synthetic_fixture_not_historical_reproduction'] = True
    report['sampling_limits'] = 'Each column is captured separately; no global receive barrier is inferred.'
    command('tp ClimbingProbe 0.5 65 0.5')
    request('prepare', position=[.5, 65., .5])
    sample = lambda **args: request('lighting', positions=POSITIONS, **args)
    # Sample received terrain sections. Empty intermediate/upper sections may
    # have no light packet at all; record those without inventing zero or sun.
    report['initial_other_heights'] = request('lighting', positions=[[2, 32, 2], [2, 200, 2]])
    initial = wait_for(sample, known, 'initial sampled light not received')
    assert initial[TARGET_INDEX]['state']['name'] == 'minecraft:stone'
    time.sleep(1)
    initial = sample(save=True)
    assert known(initial)
    boundary = trace.mark()
    command('setblock 2 64 2 minecraft:cobblestone')
    changed = wait_for(sample, lambda v: v[TARGET_INDEX]['state']['name'] == 'minecraft:cobblestone',
                       'opaque native change not received')
    captures = [changed]
    for _ in range(19):
        time.sleep(.025)
        captures.append(sample())
    assert all(known(v) and light_values(v) == light_values(initial) for v in captures)
    assert request('saved_lighting', positions=POSITIONS) == initial
    assert 'Test passed' in command('execute if block 2 64 2 minecraft:cobblestone')
    light_frames = [f for f in trace.since(boundary) if f['phase'] == 'play'
                    and f['direction'] == 'clientbound' and f['packet_id'] == 0x24]
    headers = [light_header(f) for f in light_frames]
    retained_without_receipt = []
    for p in POSITIONS:
        relevant = [h for h in headers if h['chunk'] == [p[0] // 16, p[2] // 16]]
        bit = 1 << (p[1] // 16 + 1)
        if not any((h['sky_mask'] | h['block_mask'] | h['empty_sky_mask'] | h['empty_block_mask']) & bit
                   for h in relevant):
            retained_without_receipt.append(p)
    assert retained_without_receipt, 'all sampled light sections were redelivered; no retention qualification'
    report['checks'].append(dict(name='opaque_change_retains_received_light_including_unresent_sections',
        before=initial, snapshots=captures, snapshot_count=20, known_cells=13,
        light_packet_count=len(light_frames), light_headers=headers,
        positions_without_new_light_receipt=retained_without_receipt,
        saved_snapshot_unchanged=True))

    command('setblock 2 64 2 minecraft:glowstone')
    affected = wait_for(sample, lambda v: v[TARGET_INDEX]['state']['name'] == 'minecraft:glowstone',
                        'light source not received')
    # Sparse updates can restore changed sections, but unrelated heights remain
    # unknown after conservative neighborhood invalidation. Do not claim that
    # sequential samples describe an atomic transition.
    assert any(s['sky_light'] is None or s['block_light'] is None for s in affected)
    assert request('saved_lighting', positions=POSITIONS) == initial
    command('setblock 2 64 2 minecraft:stone')
    wait_for(sample, lambda v: v[TARGET_INDEX]['state']['name'] == 'minecraft:stone',
             'source removal not received')
    command('setblock 2 64 2 minecraft:cobblestone')
    after_equivalent = wait_for(sample, lambda v: v[TARGET_INDEX]['state']['name'] == 'minecraft:cobblestone',
                                'second opaque change not received')
    assert after_equivalent[-1]['sky_light'] is None or after_equivalent[-1]['block_light'] is None
    report['checks'].append(dict(name='genuine_change_invalidates_and_equivalent_change_does_not_restore',
        source_change=affected, after_equivalent=after_equivalent, saved_snapshot_unchanged=True))

    command('tp ClimbingProbe 512.5 65 0.5')
    wait_for(lambda: request('player'), lambda v: v['received_pose']['position'][0] > 400,
             'far teleport receipt missing')
    unloaded = wait_for(sample, lambda v: all(s['state'] is None for s in v),
                        'original columns did not unload')
    command('tp ClimbingProbe 0.5 65 0.5')
    request('prepare', position=[.5, 65., .5])
    restored = wait_for(sample, known, 'full chunk light not freshly received after reload')
    assert restored[TARGET_INDEX]['state']['name'] == 'minecraft:cobblestone'
    assert restored[TARGET_INDEX]['receive_sequence'] > after_equivalent[TARGET_INDEX]['receive_sequence']
    assert request('saved_lighting', positions=POSITIONS) == initial
    report['checks'].append(dict(name='new_chunk_receipts_restore_current_light_and_keep_historical_snapshot',
        unloaded=unloaded, restored=restored, saved_snapshot_unchanged=True))
    trace.expect_disconnect()
    request('disconnect')


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--binary', type=Path, default=REPO/'target/debug/examples/climbing_control_probe')
    parser.add_argument('--jars', type=Path)
    args = parser.parse_args()
    run('1.16.1', args.binary.resolve(), args.jars, check=check)
