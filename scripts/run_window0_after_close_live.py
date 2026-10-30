#!/usr/bin/env python3
"""Official 1.16.1: local close, player click/full contents, capture, next click."""
import argparse
import json
from pathlib import Path
import run_climbing_control as h
from run_common_native import until


def check(version, command, request, trace, report):
    assert version == '1.16.1'
    command('tp ClimbingProbe 0.5 65 0.5 0 0')
    request('prepare', position=[.5, 65., .5])
    command('clear ClimbingProbe')
    command('setblock 0 65 2 minecraft:furnace')
    command('replaceitem entity ClimbingProbe inventory.0 minecraft:wooden_pickaxe 1')

    def has_pickaxe():
        slot = request('player')['inventory']['slots'][9]
        return slot and slot['value']['kind'] == 'item' and slot['value']['item']['name'] == 'minecraft:wooden_pickaxe'

    def terminal(sent):
        result = request('inventory_send_record')
        assert result['record']['id'] == sent['id'], result
        return result if result['record']['status'] in ('received', 'diverged') else None

    until(has_pickaxe, 5)
    request('use_on_block', support=[0, 65, 2], face='North', cursor=[.5, .5, 0.])
    opened = until(lambda: (s if (s := request('screen')).get('screen') and s['screen']['full_contents_sequence'] is not None else None), 5)
    until(lambda: request('player')['inventory']['cursor'] is not None, 5)
    boundary = trace.mark()
    close = request('close_screen')
    submitted = request('capture_inventory_view')[0]
    assert close['dispatched'] and close['server_close_sequence'] is None, close
    assert submitted['capture']['screen']['player_screen']['kind'] == 'submitted_close', submitted
    assert submitted['active_screen']['kind'] == 'closed', submitted
    sent = request('send_inventory_click', slot=9)
    done = until(lambda: terminal(sent), 5)
    strict = request('inventory_record')
    assert strict['stage'] == 'observed_clicked', strict
    assert strict['legacy_reply']['accepted'] is False, strict
    closed = request('capture_inventory_view')[0]
    screen = closed['capture']['screen']
    assert screen['active_window'] == 0 and screen['player_screen']['kind'] == 'received', closed
    assert screen['screen'] is None and closed['active_screen']['kind'] == 'closed', closed
    assert closed['snapshot']['screen'] is None and closed['snapshot']['source']['kind'] == 'player', closed
    assert closed['capture']['player']['inventory']['slots'][9]['value']['kind'] == 'empty', closed
    next_send = request('send_inventory_click', slot=9)
    next_done = until(lambda: terminal(next_send), 5)
    assert next_done['record']['status'] == 'received', next_done
    until(has_pickaxe, 5)
    native = command('data get entity ClimbingProbe Inventory')
    assert 'minecraft:wooden_pickaxe' in native, native
    frames = trace.since(boundary)
    assert not any(f['phase'] == 'play' and f['direction'] == 'clientbound' and f['packet_id'] == 0x13 for f in frames), frames
    full = [f for f in frames if f['phase'] == 'play' and f['direction'] == 'clientbound' and f['packet_id'] == 0x14 and bytes.fromhex(f['body_hex'])[0] == 0]
    assert full, frames
    report['checks'].append(dict(name='window0_after_exact_local_close', opened=opened,
        close=close, submitted=submitted, sent=sent, done=done, strict=strict, closed=closed,
        next_send=next_send, next_done=next_done, native=native, player_full_packets=full, frames=frames))
    trace.expect_disconnect()
    request('disconnect')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--accept-eula', action='store_true', required=True)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--jars', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    h.ROOT = args.output.resolve()
    h.run('1.16.1', args.binary.resolve(), args.jars.resolve(), check=check)
    reports = list(h.ROOT.glob('*/report.json'))
    assert reports and all(json.loads(p.read_text())['passed'] for p in reports)
