#!/usr/bin/env python3
"""Compare the public health log with original packets from SHA-1 checked vanilla servers."""
import argparse
import json
import shutil
import struct
import sys
from pathlib import Path

sys.dont_write_bytecode = True
import run_climbing_control as harness
from run_common_native import PacketTraceProxy, until


def check(version, command, request, trace, report):
    packet_id = 0x49 if version == '1.16.1' else 0x66

    def vitals():
        log = request('log')
        assert 'error' not in log, log
        return [entry['event']['Vitals'] for entry in log['changes'] if 'Vitals' in entry['event']]

    until(lambda: vitals(), 10)
    command('difficulty easy')
    command('gamerule ' + ('naturalRegeneration' if version == '1.16.1' else 'minecraft:natural_health_regeneration') + ' false')
    cut = request('subscribe')
    assert cut['baseline']['health']['health'] == 20, cut
    # Vanilla ignores ordinary damage during the first 60 ticks after login.
    import time
    time.sleep(4)
    command('effect give ClimbingProbe minecraft:instant_damage 1 0 true')
    until(lambda: any(0 < v['health']['health'] < 20 for v in vitals()), 10)
    # Leave the subscription unread during a multi-second action interval.
    time.sleep(3)
    damaged = request('drain')
    assert any(0 < event['Vitals']['health']['health'] < 20 for event in damaged['events'] if 'Vitals' in event), damaged
    command('kill ClimbingProbe')
    until(lambda: any(v['health']['health'] == 0 for v in vitals()), 10)
    report['checks'].append(dict(name='real_damage_and_death', admission=cut, drained=damaged, health=vitals()))
    assert 'error' not in request('respawn')
    def new_world():
        log = request('log')
        worlds = [c for c in log['changes'] if 'WorldChanged' in c['event']]
        return log if len(worlds) >= 2 and vitals()[-1]['health']['health'] == 20 else None
    until(new_world, 10)
    trace.expect_disconnect()
    disconnected = request('disconnect')
    assert 'error' not in disconnected, disconnected
    log = disconnected['log']
    assert log['closed'] and log['changes'][-1]['event'] == 'Disconnected', log
    original = [f for f in trace.since(0) if f['phase'] == 'play' and f['direction'] == 'clientbound' and f['packet_id'] == packet_id]
    received = [c['event']['Vitals'] for c in log['changes'] if 'Vitals' in c['event']]
    assert len(original) == len(received), (len(original), len(received))
    for frame, entry in zip(original, received):
        body = bytes.fromhex(frame['body_hex'])
        hp = struct.unpack('>f', body[:4])[0]
        food, size = PacketTraceProxy.varint(body[4:])
        saturation = struct.unpack('>f', body[4+size:8+size])[0]
        assert entry['health'] == dict(health=hp, food=food, saturation=saturation), (frame, entry)
    assert all(a['receive_sequence'] < b['receive_sequence'] for a, b in zip(received, received[1:]))
    assert len({v['session']['connection_id'] for v in received}) == 1
    assert len({v['session']['world_generation'] for v in received}) == 2
    report['checks'].append(dict(name='all_original_health_payloads_match', count=len(received), log=log, frames=original))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--accept-eula', action='store_true', required=True)
    parser.add_argument('--version', choices=['1.16.1', '1.21.11'], required=True)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--jars', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    harness.ROOT = args.output.resolve() / 'runs'
    try:
        harness.run(args.version, args.binary.resolve(), args.jars.resolve() if args.jars else None, check=check)
    finally:
        if harness.ROOT.exists():
            for folder in harness.ROOT.iterdir():
                for name in ('report.json', 'packets.jsonl', 'probe.log', 'probe.stderr.log', 'server.log'):
                    if (folder / name).exists():
                        shutil.copyfile(folder / name, args.output / name)
