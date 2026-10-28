#!/usr/bin/env python3
"""Server-forced rotation (PLAYER_ROTATION) on a SHA-1 verified official Java 1.21.11 server.

Only original server packets and RCON `rotate` commands; no synthetic replies/cache edits.
Checks that the received rotation is reported (player state, received snapshot, Body)
and that the next movement packet of a running PlayerControl carries it instead of
the stale local aim. The server's own `Rotation` NBT is the independent oracle.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import struct
import sys

sys.dont_write_bytecode = True
import run_climbing_control as harness
from run_common_native import until

ROTATION, LOOK, POSITION_LOOK = 0x47, 0x1f, 0x1e
CONTROLS = dict(forward=0, strafe=0, jump=False, sneak=False, sprint=False)


def check(version, command, request, trace, report):
    assert version == '1.21.11'

    def native_rotation():
        text = command('data get entity ClimbingProbe Rotation')
        match = re.search(r'\[([^]]+)\]', text)
        assert match, text
        return [float(v.strip().rstrip('f')) for v in match[1].split(',')]

    def play(frames, direction, packet):
        return [f for f in frames if f['phase'] == 'play' and f['direction'] == direction
                and f['packet_id'] == packet]

    def rotate(text, expected):
        """Return the frames from the original rotate command to two moves after its echo.

        A move already in flight when the proxy forwards the rotation can still
        carry the old angles (the native client has the same race); the echo
        marks the client's receipt, so every later move must carry the rotation.
        """
        mark = trace.mark()
        command(text)

        def settled():
            frames = trace.since(mark)
            echo = play(frames, 'serverbound', LOOK)
            if not echo:
                return None
            after = [f for f in frames if f['ordinal'] > echo[0]['ordinal']]
            return frames if len(play(after, 'serverbound', POSITION_LOOK)) >= 2 else None
        frames = until(settled, 10)
        forced = play(frames, 'clientbound', ROTATION)
        assert len(forced) == 1, forced
        echo = play(frames, 'serverbound', LOOK)
        assert len(echo) == 1 and echo[0]['ordinal'] > forced[0]['ordinal'], echo
        assert echo[0]['body_hex'] == struct.pack('>ffB', *expected, 0).hex(), echo
        moves = play([f for f in frames if f['ordinal'] > echo[0]['ordinal']], 'serverbound', POSITION_LOOK)
        for move in moves:
            body = bytes.fromhex(move['body_hex'])
            # The running owner's next movement packets carry the server rotation.
            assert list(struct.unpack('>ff', body[24:32])) == expected, move
        return forced[0], frames

    command('tp ClimbingProbe .5 65 .5 0 0')
    request('prepare', position=[.5, 65., .5])
    start = request('control_start')
    assert 'error' not in start, start
    held = request('control_set', controls=dict(CONTROLS, yaw=0., pitch=0.))
    assert [held['controls']['yaw'], held['controls']['pitch']] == [0, 0], held
    request('wait', ms=300)

    # Absolute: both relative flags false.
    forced, frames = rotate('rotate ClimbingProbe 45 -30', [45., -30.])
    assert forced['body_hex'] == struct.pack('>f?f?', 45., False, -30., False).hex(), forced
    record = request('control_record')
    assert record['session_id'] == start['session_id'], record
    assert record['status']['status'] == 'running', record
    assert [record['controls']['yaw'], record['controls']['pitch']] == [45, -30], record
    player = request('player')
    assert player['rotation'] == [45, -30], player
    # Received at the packet; the owner's next tick resubmits it (Submitted).
    assert player['rotation_source']['kind'] in ('received', 'submitted'), player
    request('wait', ms=500)
    assert native_rotation() == [45., -30.]
    snapshot = request('rotation_snapshot')
    assert snapshot['rotation_correction']['rotation'] == [{'absolute': 45.}, {'absolute': -30.}], snapshot
    assert snapshot['absolute_rotation']['value'] == [45, -30], snapshot
    report['checks'].append(dict(name='absolute_forced_rotation_adopted', forced=forced, record=record,
                                 player=player, snapshot=snapshot, native=native_rotation(), frames=frames))

    # Relative: `~` is relative to the command source, and the official server
    # sends the difference from its own player rotation with both flags set.
    # Run as the player so the target is 55/-30 (delta 10/0).
    forced, frames = rotate('execute as ClimbingProbe at @s run rotate @s ~10 ~', [55., -30.])
    assert forced['body_hex'] == struct.pack('>f?f?', 10., True, 0., True).hex(), forced
    record = request('control_record')
    assert [record['controls']['yaw'], record['controls']['pitch']] == [55, -30], record
    player = request('player')
    assert player['rotation'] == [55, -30], player
    request('wait', ms=500)
    assert native_rotation() == [55., -30.]
    snapshot = request('rotation_snapshot')
    assert snapshot['rotation_correction']['rotation'][0] == {'relative': 10.}, snapshot
    assert snapshot['absolute_rotation'] == 'invalidated', snapshot
    report['checks'].append(dict(name='relative_forced_rotation_adopted', forced=forced, record=record,
                                 player=player, snapshot=snapshot, native=native_rotation(), frames=frames))
    stopped = request('control_stop')
    assert stopped['corrections'] == 0, stopped  # rotation is not a position correction

    # Body: a forced rotation during a Move lease is a Pose correction.
    command('tp ClimbingProbe .5 65 .5 0 0')
    request('prepare_pose', position=[.5, 65., .5])
    sent = request('submit', kind='move', controls=dict(CONTROLS, yaw=0., pitch=0.), ticks=200)
    request('wait', ms=500)
    assert request('status', id=sent)['status'] == 'pending'
    mark = trace.mark()
    command('rotate ClimbingProbe -90 15')
    status = until(lambda: s if (s := request('status', id=sent))['status'] != 'pending' else None, 10)
    assert status['status'] == 'corrected', status
    correction = status['correction']['Pose']
    assert correction['rotation']['value'] == [-90, 15], status
    assert 'Received' in correction['rotation']['source'], status
    assert status['reason'] == 'server rotation correction superseded the action', status
    report['checks'].append(dict(name='body_move_reports_forced_rotation', status=status,
                                 frames=play(trace.since(mark), 'clientbound', ROTATION)))
    request('release')
    trace.expect_disconnect()
    request('disconnect')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--accept-eula', action='store_true', required=True)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--jars', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    harness.ROOT = args.output.resolve()
    harness.run('1.21.11', args.binary.resolve(), args.jars.resolve(), check=check)
    report_path = sorted(harness.ROOT.glob('*/report.json'))[-1]
    report = json.loads(report_path.read_text())
    sources = ['src/versions/java_1_21_11/client.rs', 'src/versions/java_1_21_11/client/correction.rs',
               'src/versions/java_1_21_11/client/motion.rs', 'src/versions/java_1_21_11/client/operations/body.rs',
               'src/versions/java_1_21_11/client/operations/continuous.rs', 'src/client/control.rs',
               'src/client/received_snapshot.rs', 'src/body/headless.rs', 'examples/body_probe.rs',
               'scripts/run_player_rotation_live.py', 'Cargo.lock']
    report['binary_sha256'] = hashlib.sha256(args.binary.read_bytes()).hexdigest()
    report['source_sha256'] = {p: hashlib.sha256((harness.REPO / p).read_bytes()).hexdigest() for p in sources}
    (harness.ROOT / 'report.json').write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
    if not report['passed']:
        raise SystemExit(1)
