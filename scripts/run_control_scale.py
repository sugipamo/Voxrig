#!/usr/bin/env python3
"""Fence held controls on out-of-range and changed own-player scale receipts."""
import argparse
import functools
import hashlib
from pathlib import Path
import struct
import subprocess
from run_climbing_control import REPO, run
from run_common_native import PacketTraceProxy, until


def check(version, command, request, trace, report, sdk):
    report['sdk'] = sdk
    def save(name, **facts):
        report['checks'].append(dict(name=name, **facts))
    def mode_ready(mode):
        return until(lambda: (p if (p := request('player'))['game_mode'] == mode else None), 5)
    def keys(flight=False):
        return dict(forward=1, strafe=0, jump=flight, sneak=not flight,
                    sprint=False, yaw=0., pitch=0.)
    def start(mode, flight):
        return request('flight_start' if flight else 'mode_start', mode=mode,
                       **({'controls': keys(True)} if flight else {}))
    def admitted(result):
        assert result['admitted'], result
        return result['record']
    def scale(value):
        boundary = request('player')['receive_sequence']
        command(f'attribute ClimbingProbe minecraft:scale base set {value}')
        def applied():
            player = request('player')
            reading = player['attributes'].get('minecraft:scale')
            return (player, reading) if reading and reading['source']['kind'] == 'received' and reading['source']['sequence'] > boundary and reading['value']['value'] == value else None
        return until(applied, 5)
    def original_scale(reading, player):
        assert reading['source']['kind'] == 'received'
        sequence = reading['source']['sequence']
        packets = [p for p in trace.since(0) if p['direction'] == 'clientbound'
                   and p['phase'] in ('configuration', 'play')]
        original = packets[sequence - 1]
        assert original['packet_id'] == 0x81 and original['phase'] == 'play'
        body = bytes.fromhex(original['body_hex'])
        entity, offset = PacketTraceProxy.varint(body)
        assert entity == player['entity_id']
        # Console base changes send this one attribute without modifiers.
        assert body[offset:] == bytes([1, 25]) + struct.pack('>d', reading['value']['base']) + b'\x00'
        assert reading['value']['modifiers'] == []
        return original
    scenarios = [('survival', False), ('adventure', False), ('creative', False),
                 ('creative', True), ('spectator', True)]
    if version == '1.16.1':
        for mode, flight in [('survival', False), ('creative', True)]:
            command(f'gamemode {mode} ClimbingProbe'); mode_ready(mode)
            position = [.5, 72. if flight else 65., .5]
            command(f'tp ClimbingProbe 0.5 {position[1]} 0.5 0 0')
            request('prepare', position=position)
            if flight:
                admitted(request('flight_enable', mode=mode, flying=True))
            owner = admitted(start(mode, flight))
            record = request('ticks', count=3)
            assert record['status']['status'] == 'running' and record['scale_change_sequence'] is None and not record['corrections']
            stopped = admitted(request('mode_stop', mode=mode, session_id=owner['session_id']))
            save(mode + '_unit_scale_regression', record=record, stopped=stopped)
        report['scale_attribute_support'] = False
    else:
        report['scale_attribute_support'] = True
        for mode, flight in scenarios:
            name = mode + ('_flight' if flight else '_ground')
            command(f'gamemode {mode} ClimbingProbe'); mode_ready(mode)
            position = [.5, 72. if flight else 65., .5]
            command(f'tp ClimbingProbe 0.5 {position[1]} 0.5 0 0')
            request('prepare', position=position)
            if flight and mode == 'creative':
                admitted(request('flight_enable', mode=mode, flying=True))
            player, reading = scale(16)
            refused = start(mode, flight)
            assert not refused['admitted'] and 'unmodeled player scale' in refused['error'], refused
            save(name + '_scaled_admission_refused', refusal=refused,
                 original_attribute=original_scale(reading, player))
            scale(1)
            owner = admitted(start(mode, flight))
            if not flight:
                admitted(request('mode_keys', mode=mode, session_id=owner['session_id'], controls=keys()))
            request('ticks', count=3)
            boundary = trace.mark()
            player, reading = scale(2)
            stopped = until(lambda: (r if (r := request('record'))['status']['status'] == 'stopped' else None), 5)
            assert stopped['status']['reason'] == 'unmodeled player scale', stopped
            assert stopped['scale_change_sequence'] == reading['source']['sequence']
            assert not stopped['corrections'], stopped
            release = until(lambda: [p for p in trace.since(boundary) if p['direction'] == 'serverbound' and p['phase'] == 'play' and p['packet_id'] == 0x2a and p.get('body_hex') == '00'], 5)
            save(name + '_received_scale_stops_releases', record=stopped,
                 original_attribute=original_scale(reading, player), neutral_packets=release)
            scale(1)
            assert request('wait', ms=150) == stopped
            stale = request('mode_keys', mode=mode, session_id=owner['session_id'], controls=keys(flight))
            assert not stale['admitted'], stale
            save(name + '_restoration_retains_owner', record=stopped, stale_refusal=stale)
            fresh = admitted(start(mode, flight))
            assert fresh['session_id'] != owner['session_id'] and fresh['scale_change_sequence'] is None
            admitted(request('mode_stop', mode=mode, session_id=fresh['session_id']))
            save(name + '_explicit_unit_scale_restart', record=fresh)
    trace.expect_disconnect()
    request('disconnect')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--accept-eula', action='store_true', required=True)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--jars', type=Path, default=REPO / '.local/climbing/downloads')
    parser.add_argument('--version', choices=['1.16.1', '1.21.11'], action='append')
    parser.add_argument('--compiled-sdk-revision', required=True)
    args = parser.parse_args()
    revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=REPO, text=True).strip()
    compiled = subprocess.check_output(['git', 'rev-parse', args.compiled_sdk_revision], cwd=REPO, text=True).strip()
    paths = ['src', 'examples', 'Cargo.toml', 'Cargo.lock', 'data']
    assert not subprocess.check_output(['git', 'diff', compiled, 'HEAD', '--', *paths], cwd=REPO)
    assert not subprocess.check_output(['git', 'diff', 'HEAD', '--', *paths], cwd=REPO)
    files = ['Cargo.lock', 'scripts/run_control_scale.py', 'scripts/run_climbing_control.py',
             'scripts/run_common_native.py', 'examples/climbing_control_probe.rs']
    sdk = dict(source_revision=revision, binary_build_revision=compiled,
               binary_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest(),
               artifacts={f: hashlib.sha256((REPO / f).read_bytes()).hexdigest() for f in files})
    for version in args.version or ['1.16.1', '1.21.11']:
        run(version, args.binary.resolve(), args.jars.resolve(),
            check=functools.partial(check, sdk=sdk))


if __name__ == '__main__':
    main()
