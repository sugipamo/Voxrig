#!/usr/bin/env python3
"""Check held honey contact/exit, crouching and landing on a disposable server."""
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
    artifacts = ['src/client/physics/mod.rs', 'src/client/physics/bubbles.rs',
                 'src/versions/table.rs', 'Cargo.lock', 'examples/climbing_control_probe.rs',
                 'scripts/run_honey_live.py', 'scripts/run_climbing_control.py',
                 'scripts/run_common_native.py', 'data/client_api/honey_oracle.json.gz']
    report['source_sha256'] = {f: hashlib.sha256((repo/f).read_bytes()).hexdigest() for f in artifacts}


    def admitted(result):
        assert result['admitted'], result
        return result['record']

    def keys(**changes):
        return dict(dict(forward=0, strafe=0, jump=False, sneak=False, sprint=False,
                         yaw=0., pitch=0.), **changes)

    def capture(name, record, boundary):
        assert record['status']['status'] == 'running', record
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
        if name == 'crouch_slide':
            inputs = [f for f in trace.since(boundary) if f['direction']=='serverbound'
                      and f['phase']=='play' and f['packet_id']==0x2a]
            assert any(int(f['body_hex'], 16) & 32 for f in inputs), inputs
            report['checks'][-1]['original_sneak_inputs'] = inputs
        return frame

    command('effect give ClimbingProbe minecraft:resistance 600 4 true')
    command('fill -4 65 -4 4 84 4 minecraft:air')
    command('fill 1 65 -3 1 80 3 minecraft:honey_block')
    for sneak in (False, True):
        position = [.7625 if sneak else .5, 69., .5]
        command('tp ClimbingProbe ' + ' '.join(map(str, position)) + ' 0 0')
        request('prepare', position=position)
        boundary = trace.mark()
        owner = admitted(request('mode_start', mode='survival'))
        admitted(request('mode_keys', mode='survival', session_id=owner['session_id'],
                         controls=keys(sneak=sneak, strafe=0 if sneak else 1)))
        frame = capture('crouch_slide' if sneak else 'fall_contact', request('ticks', count=10), boundary)
        assert frame['velocity'][1] == (-.05-.08)*struct.unpack('>f',struct.pack('>f',.98))[0], frame
        assert not frame['on_ground'], frame
        if sneak:
            landing = capture('wall_landing', request('ticks', count=40), boundary)
            assert landing['on_ground'] and abs(landing['position'][1]-65.)<1e-8, landing
        else:
            admitted(request('mode_keys', mode='survival', session_id=owner['session_id'],
                             controls=keys(strafe=-1)))
            departed = capture('leave_wall', request('ticks', count=6), boundary)
            assert departed['position'][0] < frame['position'][0]-.3, departed
            assert departed['velocity'][1] < frame['velocity'][1], departed
            landing = capture('detached_landing', request('ticks', count=12), boundary)
            assert landing['on_ground'], landing
        stopped = admitted(request('mode_stop', mode='survival', session_id=owner['session_id']))
        assert request('wait', ms=100) == stopped
        report['checks'].append(dict(name='crouch_stop' if sneak else 'detached_stop', record=stopped))
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
