#!/usr/bin/env python3
"""Verify that a running 1.16.1 Ground control keeps explosion knockback (audit P10).

The explosion is a real creeper on an isolated official server. Original
clientbound Explosion bytes, the serverbound movement packets that follow,
ControlRecord frames and RCON positions are recorded as separate evidence.
"""
import argparse
import hashlib
import json
import re
import struct
from pathlib import Path
import run_climbing_control as live
from run_common_native import until

SOURCES = ['src/client/control.rs', 'src/versions/java_1_16_1/client/common_control.rs',
           'src/versions/java_1_16_1/client/play_world.rs',
           'examples/climbing_control_probe.rs', 'scripts/run_control_explosion_live.py', 'Cargo.lock']
EXPLOSION, MOVE = 0x1c, 0x13


def check(version, command, request, trace, report):
    assert version == '1.16.1'
    repo = Path(__file__).resolve().parents[1]
    report['source_sha256'] = {p: hashlib.sha256((repo/p).read_bytes()).hexdigest() for p in SOURCES}

    def admitted(response):
        assert response['admitted'], response
        return response['record']

    def keys(**changes):
        return dict(dict(forward=0, strafe=0, jump=False, sneak=False, sprint=False, yaw=0., pitch=0.), **changes)

    def position():
        text = command('data get entity ClimbingProbe Pos')
        match = re.search(r'\[([^]]+)\]', text)
        assert match, text
        return [float(x.strip().removesuffix('d')) for x in match[1].split(',')]

    def moves(mark):
        return [dict(f, decoded=struct.unpack('>dddff?', bytes.fromhex(f['body_hex'])))
                for f in trace.since(mark) if f['direction'] == 'serverbound'
                and f['phase'] == 'play' and f['packet_id'] == MOVE]

    def explode(name):
        after = request('explosions')['receive_sequence']
        before = request('record')
        mark = trace.mark()
        # Creeper respects mobGriefing=false, so the floor stays known.
        command('execute at ClimbingProbe run summon minecraft:creeper ~1.5 ~ ~ '
                '{Fuse:1s,ignited:1b,NoAI:1b,ExplosionRadius:2b}')
        history = until(lambda: (lambda h: h if h['explosions'] else None)(request('explosions', after=after)), 5)
        assert len(history['explosions']) == 1, history
        knockback = history['explosions'][0]['explosion']['player_knockback']
        assert knockback is not None and any(abs(v) > .05 for v in knockback), history
        record = request('ticks', count=6)
        frames = trace.since(mark)
        original = [f for f in frames if f['direction'] == 'clientbound' and f['phase'] == 'play'
                    and f['packet_id'] == EXPLOSION]
        assert len(original) == 1, frames
        after_moves = [m for m in moves(mark) if m['ordinal'] > original[0]['ordinal']]
        assert after_moves, frames
        report['checks'].append(dict(name=name, before=before, observations=history,
                                     original_explosion=original, movement_after=after_moves,
                                     record=record, native_position=position()))
        return knockback, record, after_moves

    command('difficulty normal')
    command('gamerule mobGriefing false')
    command('effect give ClimbingProbe minecraft:resistance 1000000 255 true')
    command('fill -12 64 -12 12 64 12 minecraft:stone')
    command('fill -12 65 -12 12 84 12 minecraft:air')
    command('tp ClimbingProbe 0.5 65 0.5 0 0')
    request('prepare', position=[.5, 65., .5])
    owner = admitted(request('mode_start', mode='survival'))
    standing = request('ticks', count=3)
    assert standing['frame']['position'] == [.5, 65., .5], standing
    assert standing['explosion_impulses'] == 0, standing
    report['checks'].append(dict(name='standing_before', record=standing, native_position=position()))

    knockback, record, after_moves = explode('standing_explosion')
    assert record['session_id'] == owner['session_id'], record
    assert record['status']['status'] == 'running', record
    assert record['explosion_impulses'] == 1 and record['corrections'] == 0, record
    # The impulse reaches the next movement packets instead of being overwritten.
    assert max(abs(m['decoded'][0] - .5) + abs(m['decoded'][2] - .5) for m in after_moves) > .05, after_moves
    assert any(m['decoded'][1] > 65. for m in after_moves), after_moves

    landed = until(lambda: (lambda r: r if r['frame']['on_ground'] else None)(request('ticks', count=2)), 10)
    native = position()
    assert max(abs(a-b) for a, b in zip(native, landed['frame']['position'])) < .05, (native, landed)
    assert landed['explosion_impulses'] == 1 and landed['corrections'] == 0, landed
    report['checks'].append(dict(name='landed_once', record=landed, native_position=native))

    # A second explosion is counted again, still without a server correction.
    _, second, _ = explode('second_explosion')
    assert second['explosion_impulses'] == 2 and second['corrections'] == 0, second
    assert second['session_id'] == owner['session_id'], second
    stopped = admitted(request('mode_stop', mode='survival', session_id=owner['session_id']))
    assert stopped['explosion_impulses'] == 2, stopped
    report['checks'].append(dict(name='stopped_record_retains_count', record=stopped))
    trace.expect_disconnect()
    request('disconnect')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--jars', type=Path, required=True)
    parser.add_argument('--work', type=Path, required=True)
    parser.add_argument('--accept-eula', action='store_true', required=True)
    args = parser.parse_args()
    live.ROOT = args.work.resolve()
    binary = args.binary.resolve()
    before = hashlib.sha256(binary.read_bytes()).hexdigest()
    live.run('1.16.1', binary, args.jars.resolve(), check=check)
    assert hashlib.sha256(binary.read_bytes()).hexdigest() == before
    print(json.dumps(dict(binary_sha256=before)))


if __name__ == '__main__':
    main()
