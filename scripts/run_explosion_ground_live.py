#!/usr/bin/env python3
"""Verify Ground explosion continuation and once-only restart on an isolated server."""
import argparse
import hashlib
import json
import re
import struct
from pathlib import Path
import run_climbing_control as live
from run_common_native import until


def check(version, command, request, trace, report):
    assert version == '1.21.11'
    repo = Path(__file__).resolve().parents[1]
    sources = ['src/client/control.rs', 'src/versions/java_1_21_11/client.rs',
               'src/versions/java_1_21_11/client/operations/continuous.rs',
               'src/versions/java_1_21_11/client/operations/survival.rs',
               'examples/climbing_control_probe.rs', 'scripts/run_explosion_ground_live.py',
               'data/client_api/explosion_ground_oracle.json.gz', 'Cargo.lock']
    report['source_sha256'] = {p: hashlib.sha256((repo/p).read_bytes()).hexdigest() for p in sources}

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

    def capture(name, record):
        assert record['status']['status'] == 'running', record
        frame = record['frame']
        native = position()
        assert max(abs(a-b) for a,b in zip(native, frame['position'])) < .8, (native, frame)
        expected = struct.pack('>dddff', *frame['position'], record['controls']['yaw'], record['controls']['pitch'])
        expected += bytes([int(frame['on_ground']) | int(frame['horizontal_collision']) << 1])
        packets = until(lambda: [f for f in trace.since(boundary) if f['direction']=='serverbound'
                                 and f['phase']=='play' and f['packet_id']==0x1e
                                 and f['body_hex']==expected.hex()], 5)
        report['checks'].append(dict(name=name, record=record, native_position=native, movement=packets[-1]))
        return frame

    def explode(name, airborne=False):
        after = request('explosions')['receive_sequence']
        mark = trace.mark()
        # Creeper respects mob_griefing=false; the floor remains independently known.
        command('execute at ClimbingProbe run summon minecraft:creeper ~1.5 ~ ~ {Fuse:1s,ignited:1b,NoAI:1b,ExplosionRadius:1b}')
        history = until(lambda: (lambda h: h if h['explosions'] else None)(request('explosions', after=after)), 5)
        assert len(history['explosions']) == 1, history
        assert history['explosions'][0]['explosion']['player_knockback'] is not None, history
        assert request('explosions', after=after)['explosions'] == history['explosions']
        packets = [f for f in trace.since(mark) if f['direction']=='clientbound' and f['phase']=='play']
        air_move = None
        if airborne:
            explosion_packets = [f for f in packets if f['packet_id']==0x24]
            assert len(explosion_packets)==1, packets
            before = [f for f in trace.since(boundary) if f['direction']=='serverbound'
                      and f['phase']=='play' and f['packet_id']==0x1e
                      and f['ordinal'] < explosion_packets[0]['ordinal']]
            assert before, packets
            air_move = before[-1]
            assert struct.unpack_from('>d',bytes.fromhex(air_move['body_hex']),8)[0] > 65., air_move
        report['checks'].append(dict(name=name, observations=history, original_packets=packets,
                                     airborne_movement_before_receipt=air_move))
        return history

    command('difficulty normal')
    command('gamerule minecraft:mob_griefing false')
    command('effect give ClimbingProbe minecraft:resistance infinite 255 true')
    command('fill -12 64 -12 12 64 12 minecraft:stone')
    command('fill -12 65 -12 12 84 12 minecraft:air')
    command('tp ClimbingProbe 0.5 65 0.5 0 0')
    request('prepare', position=[.5,65.,.5])
    boundary = trace.mark()
    owner = admitted(request('mode_start', mode='survival'))
    admitted(request('mode_keys',mode='survival',session_id=owner['session_id'],controls=keys(forward=1)))
    capture('walking_before',request('ticks',count=3))
    explode('walking_explosion')
    walking = request('ticks',count=2)
    capture('walking_after',walking)
    assert walking['corrections'] == 0, walking
    admitted(request('mode_keys',mode='survival',session_id=owner['session_id'],controls=keys()))
    # Use an explicit jump so network/RCON latency cannot turn the airborne case
    # into a weak first explosion that has already landed.
    admitted(request('mode_keys',mode='survival',session_id=owner['session_id'],controls=keys(jump=True)))
    jumping = capture('airborne_before',request('ticks',count=2))
    assert jumping['position'][1] > 65., jumping
    admitted(request('mode_keys',mode='survival',session_id=owner['session_id'],controls=keys()))
    explode('airborne_second_explosion',airborne=True)
    air = request('ticks',count=2)
    capture('airborne_after',air)
    assert air['session_id']==owner['session_id'] and air['corrections']==0, air
    restarted = request('restart')
    assert restarted['first']['dispatched_ticks']==1, restarted
    stopped, first = restarted['stopped']['frame'], restarted['first']['frame']
    expected = [a+b for a,b in zip(stopped['position'],stopped['velocity'])]
    expected[1] = max(65.,expected[1])
    assert max(abs(a-b) for a,b in zip(first['position'],expected)) < 1e-8, restarted
    report['checks'].append(dict(name='stop_restart_no_impulse_replay', result=restarted))
    owner = restarted['started']
    capture('resumed_after_explosions',request('ticks',count=2))
    # A real server correction replaces the old velocity before later impulses.
    command('tp ClimbingProbe 0.5 65 0.5 0 0')
    corrected = until(lambda: (lambda r: r if r['status']['status']=='stopped' else None)(request('record')),5)
    assert corrected['corrections']==1, corrected
    assert 'own correction discarded contact origin' in corrected['status']['reason'], corrected
    report['checks'].append(dict(name='correction_retains_contact_diagnosis',record=corrected,
                                 native_position=position(),player=request('player')))
    owner = admitted(request('mode_start',mode='survival'))
    assert owner['session_id'] != corrected['session_id'], (owner,corrected)
    fresh = request('ticks',count=2)
    capture('fresh_owner_after_correction',fresh)
    assert fresh['corrections']==0, fresh
    explode('explosion_after_correction')
    corrected = request('ticks',count=3)
    capture('continued_after_correction_explosion',corrected)
    assert corrected['corrections']==0 and corrected['session_id']==owner['session_id'], corrected
    stopped = admitted(request('mode_stop',mode='survival',session_id=owner['session_id']))
    history = request('explosions')
    assert len(history['explosions'])==3
    assert request('wait',ms=120)==stopped
    assert request('explosions')['explosions']==history['explosions']
    report['checks'].append(dict(name='stopped_history_and_frame_retained',record=stopped,observations=history))
    trace.expect_disconnect()
    request('disconnect')


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary',type=Path,required=True)
    parser.add_argument('--jars',type=Path,required=True)
    parser.add_argument('--work',type=Path,required=True)
    parser.add_argument('--accept-eula',action='store_true',required=True)
    args=parser.parse_args()
    live.ROOT=args.work.resolve()
    binary=args.binary.resolve()
    before=hashlib.sha256(binary.read_bytes()).hexdigest()
    live.run('1.21.11',binary,args.jars.resolve(),check=check)
    assert hashlib.sha256(binary.read_bytes()).hexdigest()==before
    print(json.dumps(dict(binary_sha256=before)))


if __name__=='__main__':
    main()
