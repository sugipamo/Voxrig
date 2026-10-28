#!/usr/bin/env python3
"""Exercise received-view capacity on an official 1.16.1 server.

The trace proxy withholds original UNLOAD_CHUNK packets to make capacity pressure
repeatable across teleports. Every other frame is forwarded unchanged. This is
a missing-unload fixture, not a claim about vanilla unload ordering. Server
commands and inventory/world truth come from independent RCON. No chunk packet
is synthesized, edited or replayed.
"""
import argparse
import json
from pathlib import Path

import run_climbing_control as harness
from run_common_native import PacketTraceProxy, until


class DeferredUnloads(PacketTraceProxy):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self.withheld = 0

    def forward_frame(self, record, destination, wire):
        if (record['phase'] == 'play' and record['direction'] == 'clientbound'
                and record['packet_id'] == 0x1d):
            self.withheld += 1
            return
        super().forward_frame(record, destination, wire)


def check(version, command, request, trace, report):
    assert version == '1.16.1'
    command('tp ClimbingProbe 0.5 65 0.5 0 0')
    request('prepare', position=[.5, 65., .5])
    # Cross a column boundary using the actual continuous motion writer first.
    command('fill -4 64 -4 4 64 40 minecraft:stone')
    request('start')
    request('keys', controls=dict(forward=1, strafe=0, jump=False,
                                 sneak=False, sprint=False, yaw=0., pitch=0.))
    request('ticks', count=80)
    movement = request('stop')
    native_motion = command('data get entity ClimbingProbe Pos')
    report['checks'].append(dict(name='continuous_movement', record=movement,
                                 native_position=native_motion))
    view = until(lambda: (c if (c := request('chunk_cache'))['view_distance'] else None), 15)
    assert view['view_distance'] == 16, view
    assert view['capacity'] == 2 * (2 * 16 + 3) ** 2, view
    view = until(lambda: (c if (c := request('chunk_cache'))['columns'] > 512 else None), 120)
    original = request('chunk_probe', position=[0, 64, 0])
    assert original['known'] and original['state'] is not None, original
    previous = original['cache']
    # Three disjoint received views exceed the old/new two-view budget when
    # unload delivery is absent. The own received view and local body stay pinned.
    for x in (768, 1536, 2304):
        command(f'forceload add {x-16} -16 {x+16} 16')
        command(f'fill {x-4} 64 -4 {x+4} 64 4 minecraft:stone')
        command(f'tp ClimbingProbe {x+.5} 65 0.5 0 0')
        until(lambda: abs(request('player')['received_pose']['position'][0] - (x+.5)) < .01, 20)
        until(lambda: request('chunk_probe', position=[x,64,0])['known'], 30)
        # Wait for distant cardinal terrain receipts, not a quiet instant while
        # the server is still generating its next batch.
        until(lambda: all(request('chunk_probe', position=p)['known'] for p in (
            [x+15*16,64,0], [x-15*16,64,0], [x,64,15*16], [x,64,-15*16])), 120)
        request('wait', ms=3000)
        cache = request('chunk_cache')
        near = request('chunk_probe', position=[x,64,0])
        assert near['known'] and near['state'] is not None, near
        assert cache['columns'] <= cache['capacity'], cache
        assert cache['refusal'] is None, cache
        report['checks'].append(dict(name='teleport', x=x, cache=cache,
                                     body_column=near, native_position=command('data get entity ClimbingProbe Pos')))
    final = until(lambda: (c if (c := request('chunk_cache'))['evicted_columns'] > 0 else None), 20)
    old = until(lambda: (p if not (p := request('chunk_probe', position=[0,64,0]))['known'] else None), 20)
    assert old['state'] is None and old['context'] is None, old
    assert final['epoch'] != previous['epoch'], (previous, final)
    assert final['session'] == previous['session'], (previous, final)
    assert final['last_eviction']['view_distance'] == 16, final
    assert final['last_eviction']['capacity'] == final['capacity'], final
    assert trace.withheld > 0
    request('look', rotation=[90., 0.])
    report['checks'].append(dict(name='evicted_unknown_and_epoch_invalidated', before=original,
                                 after=old, final=final, withheld_unloads=trace.withheld,
                                 connected_player=request('player')))
    trace.expect_disconnect()
    request('disconnect')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--accept-eula', action='store_true', required=True)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--jars', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    harness.ROOT = args.output.resolve()
    harness.run('1.16.1', args.binary.resolve(), args.jars.resolve(), check=check,
                trace_factory=DeferredUnloads, server_properties={'view-distance': 16})
    reports = list(harness.ROOT.glob('*/report.json'))
    if not reports or not all(json.loads(p.read_text())['passed'] for p in reports):
        raise SystemExit(1)


if __name__ == '__main__':
    main()
