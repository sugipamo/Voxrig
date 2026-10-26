#!/usr/bin/env python3
"""Finite strict interruption/handoff on SHA-1 checked official vanilla servers.

Use the existing original-byte proxy and RCON harness. No packets/cache edited.
"""
import argparse
import hashlib
import json
import re
import shutil
import struct
import threading
from pathlib import Path

import run_climbing_control as harness
from run_common_native import until


def check(version, command, request, trace, report):
    report['binary_sha256'] = hashlib.sha256(BINARY.read_bytes()).hexdigest()
    # Pinned native POSITION_LOOK IDs, shared with run_common_native fixtures.
    movement = 0x13 if version == '1.16.1' else 0x1e

    def pos():
        raw = command('data get entity ClimbingProbe Pos')
        return [float(v.strip().rstrip('df')) for v in re.search(r'\[([^]]+)\]', raw)[1].split(',')]

    def controls(n=35, moving=10, yaw=0.):
        return [dict(yaw=yaw, input=dict(forward=int(t < moving), strafe=0, jump=False)) for t in range(n)]

    def prepared():
        command('fill -5 64 -5 15 64 15 minecraft:stone')
        command('fill -4 65 -4 14 70 14 minecraft:air')
        command('gamemode survival ClimbingProbe')
        command('tp ClimbingProbe 0.5 65 0.5 0 0')
        request('prepare', position=[.5,65.,.5])
        until(lambda: request('player')['game_mode'] == 'survival')
        request('wait', ms=300)

    for cause in ['correction', 'terrain', 'world']:
        prepared()
        if cause == 'world':
            # Load/build the destination before starting the 1.75 second run.
            command('execute in minecraft:the_nether run forceload add -8 -8 8 8')
            request('wait', ms=2000)
            command('execute in minecraft:the_nether run fill -3 64 -3 8 64 8 minecraft:stone')
            command('execute in minecraft:the_nether run fill -2 65 -2 7 70 7 minecraft:air')
        hit = threading.Event()
        count = 0
        mark = trace.mark()
        def tick(frame):
            nonlocal count
            if frame['phase'] == 'play' and frame['direction'] == 'serverbound' and frame['packet_id'] == movement:
                count += 1
                if count == 3:
                    hit.set()
        trace.before_forward = tick
        started = request('finite_start', controls=controls())
        assert 'error' not in started, (cause, started)
        assert hit.wait(5), 'third finite frame never reached proxy'
        if cause == 'correction':
            command('tp ClimbingProbe 8.5 65 0.5 0 0')
        elif cause == 'terrain':
            command('setblock 0 65 2 minecraft:stone')
        else:
            command('execute in minecraft:the_nether run tp ClimbingProbe 0.5 65 0.5 0 0')
        ended = until(lambda: (r if r and r['status'] == 'requires_inspection' else None)
                      if (r := request('finite_record')) else None, 8)
        assert ended['dispatched_ticks'] < 35, (cause, ended)
        # Allow the owner to finish neutral write/actor acknowledgement, not a server rest echo.
        request('wait', ms=120)
        old_frames = trace.since(mark)
        old_tail = {tuple(f['position']) for f in ended['preview']['frames'][ended['dispatched_ticks']:]}
        def positions_in(frames):
            return [list(struct.unpack_from('>ddd', bytes.fromhex(f['body_hex']))) for f in frames
                    if f['phase'] == 'play' and f['direction'] == 'serverbound' and f['packet_id'] == movement]
        assert not any(tuple(p) in old_tail for p in positions_in(old_frames)), ('old tail sent before handoff', cause)
        trace.before_forward = lambda frame: None
        before = pos()
        path = controls(n=5, moving=5, yaw=-90.)
        fresh = until(lambda: (r if 'record' in r else None)
                      if (r := request('path_best_effort', controls=path)) else None, 8)
        request('wait', ms=500)
        current = request('control_record')
        after = pos()
        assert after[0] > before[0] + .10, (cause, before, after, fresh, current)
        # Every post-interruption coordinate uses the corrected/current model.
        later = trace.since(mark + len(old_frames))
        positions = positions_in(later)
        assert positions, 'no new movement packets'
        assert not any(tuple(p) in old_tail for p in positions), ('old tail reappeared', cause, positions)
        if cause == 'correction':
            assert all(p[0] > 8. for p in positions), positions
        # Fresh Body movement and strict neutral settle on this same connection.
        request('control_stop')
        body = request('submit', kind='move', controls=dict(forward=0, strafe=1, jump=False,
                       sneak=False, sprint=False, yaw=0., pitch=0.), ticks=2)
        assert 'error' not in body, body
        terminal = until(lambda: (s if s['status'] != 'pending' else None)
                         if (s := request('status', id=body)) else None, 5)
        assert terminal['status'] == 'accepted', terminal
        request('wait', ms=200)
        strict = request('finite_start', controls=controls(n=35, moving=0))
        assert 'error' not in strict, (cause, strict)
        done = until(lambda: (r if r and r['status'] != 'running' else None)
                     if (r := request('finite_record')) else None, 8)
        assert done['status'] == 'predicted', (cause, done)
        report['checks'].append(dict(name=cause, interrupted=ended, before=before, after=after,
            best_effort=fresh, body=terminal, next_strict=done,
            movement_frames=count, old_frames=old_frames, next_positions=positions))
        if cause == 'world':
            command('execute in minecraft:overworld run tp ClimbingProbe 0.5 65 0.5')
            request('prepare', position=[.5,65.,.5])
    trace.expect_disconnect()
    request('disconnect')


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--accept-eula', action='store_true', required=True)
    p.add_argument('--version', choices=['1.16.1','1.21.11'], required=True)
    p.add_argument('--binary', type=Path, required=True)
    p.add_argument('--jars', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    args = p.parse_args()
    BINARY = args.binary.resolve()
    args.output.mkdir(parents=True, exist_ok=True)
    harness.ROOT = args.output.resolve() / 'runs'
    try:
        harness.run(args.version, BINARY, args.jars.resolve(), check=check)
    finally:
        if harness.ROOT.exists():
            for folder in harness.ROOT.iterdir():
                for name in ('report.json','packets.jsonl','probe.log','probe.stderr.log','server.log'):
                    if (folder/name).exists(): shutil.copyfile(folder/name,args.output/name)
