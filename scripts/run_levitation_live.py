#!/usr/bin/env python3
"""Check received levitation changes and Ground continuation on a disposable server."""
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
    artifacts = ['src/client/physics/mod.rs', 'src/client/control.rs',
                 'src/versions/java_1_21_11/client/operations/continuous.rs',
                 'Cargo.lock', 'examples/climbing_control_probe.rs',
                 'scripts/run_levitation_live.py', 'scripts/run_climbing_control.py',
                 'scripts/run_common_native.py', 'data/client_api/levitation_oracle.json.gz']
    report['source_sha256'] = {f: hashlib.sha256((repo/f).read_bytes()).hexdigest() for f in artifacts}

    def admitted(result):
        assert result['admitted'], result
        return result['record']

    def keys(**changes):
        return dict(dict(forward=0, strafe=0, jump=False, sneak=False, sprint=False,
                         yaw=0., pitch=0.), **changes)

    def effect(name, amplifier):
        boundary = trace.mark()
        if amplifier is None:
            command('effect clear ClimbingProbe ' + name)
        else:
            command(f'effect give ClimbingProbe {name} 600 {amplifier} true')
        def received():
            player = request('player')
            value = player['effects'].get(name)
            return player if ((value is None) if amplifier is None else
                              value and value['value']['amplifier'] == amplifier) else None
        player = until(received, 5)
        # Keep actual receipts and original bytes separate from the model.
        packets = [f for f in trace.since(boundary) if f['direction']=='clientbound'
                   and f['phase']=='play']
        report['checks'].append(dict(name='effect_' + name.split(':')[1] + '_' + str(amplifier),
                                     player=player, original_packets=packets))

    def capture(name, record):
        assert record['status']['status'] == 'running', record
        assert record['session_id'] == owner['session_id'], record
        assert not record['corrections'], record
        count = record['dispatched_ticks']
        def moves():
            frames = [f for f in trace.since(boundary) if f['direction']=='serverbound'
                      and f['phase']=='play' and f['packet_id']==0x1e]
            return frames if len(frames) >= count else None
        packets = until(moves, 5)
        frame = record['frame']
        flags = int(frame['on_ground']) | int(frame['horizontal_collision']) << 1
        expected = struct.pack('>dddff', *frame['position'], record['controls']['yaw'],
                               record['controls']['pitch']) + bytes([flags])
        assert packets[count-1]['body_hex'] == expected.hex(), (packets[count-1], record)
        response = command('data get entity ClimbingProbe Pos')
        match = re.search(r'\[([^]]+)\]', response)
        assert match, response
        native = [float(s.strip().removesuffix('d')) for s in match[1].split(',')]
        assert max(abs(a-b) for a,b in zip(native, frame['position'])) < .6, (native, frame)
        report['checks'].append(dict(name=name, record=record, movement=packets[count-1],
                                     native_position=native))
        return frame

    command('effect give ClimbingProbe minecraft:resistance 600 4 true')
    command('fill -4 65 -4 4 84 4 minecraft:air')
    command('fill -4 70 -4 4 70 4 minecraft:stone')
    position = [.5, 65., .5]
    command('tp ClimbingProbe ' + ' '.join(map(str, position)) + ' 0 0')
    request('prepare', position=position)
    boundary = trace.mark()
    owner = admitted(request('mode_start', mode='survival'))
    capture('initial_ground', request('ticks', count=3))
    effect('minecraft:levitation', 0)
    admitted(request('mode_keys', mode='survival', session_id=owner['session_id'], controls=keys(forward=1)))
    rising = capture('added_during_control', request('ticks', count=12))
    assert rising['position'][1] > 65.2 and rising['velocity'][1] > 0., rising
    effect('minecraft:slow_falling', 0)
    effect('minecraft:levitation', 4)
    admitted(request('mode_keys', mode='survival', session_id=owner['session_id'], controls=keys()))
    ceiling = capture('higher_amplifier_ceiling', request('ticks', count=25))
    assert abs(ceiling['position'][1] - (70.-struct.unpack('>f',struct.pack('>f',1.8))[0])) < 1e-8, ceiling
    assert ceiling['velocity'][1] > 0., ceiling
    effect('minecraft:levitation', None)
    effect('minecraft:levitation', 1)
    admitted(request('mode_keys', mode='survival', session_id=owner['session_id'], controls=keys(strafe=1)))
    lower = capture('lower_amplifier_at_ceiling', request('ticks', count=5))
    assert lower['position'][0] > ceiling['position'][0] + .05, (lower, ceiling)
    admitted(request('mode_keys', mode='survival', session_id=owner['session_id'], controls=keys()))
    assert lower['velocity'][1] < ceiling['velocity'][1], (lower, ceiling)
    effect('minecraft:levitation', None)
    falling = capture('removed_with_slow_falling', request('ticks', count=12))
    assert falling['position'][1] < lower['position'][1] and falling['velocity'][1] < 0., falling
    effect('minecraft:slow_falling', None)
    landed = capture('normal_gravity_landing', request('ticks', count=25))
    assert landed['on_ground'], landed
    stopped = admitted(request('mode_stop', mode='survival', session_id=owner['session_id']))
    assert request('wait', ms=100) == stopped
    report['checks'].append(dict(name='stopped_record_retained', record=stopped))
    # Existing effect at admission uses the same received amplifier.
    effect('minecraft:levitation', 1)
    boundary = trace.mark()
    owner = admitted(request('mode_start', mode='survival'))
    second = capture('admission_with_effect', request('ticks', count=8))
    assert second['velocity'][1] > .05 and second['position'][1] > 65.1, second
    admitted(request('mode_stop', mode='survival', session_id=owner['session_id']))
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
    live.run('1.21.11', binary, args.jars.resolve(), check=check)
    assert hashlib.sha256(binary.read_bytes()).hexdigest() == before
    print(json.dumps(dict(binary_sha256=before)))


if __name__ == '__main__':
    main()
