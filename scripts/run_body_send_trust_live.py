#!/usr/bin/env python3
"""Send-and-trust on SHA-1 verified official servers, original packets and RCON.

Delays forward only original bytes; never edits SDK caches or fabricates ACKs.
"""
import argparse
import hashlib
import os
import shutil
import sys
import threading
import time
from pathlib import Path

sys.dont_write_bytecode = True

import run_climbing_control as harness
from run_common_native import until


def check(version, command, request, trace, report):
    legacy = version == '1.16.1'
    trace.record_filter = lambda frame: True  # adds monotonic timestamps without filtering bytes
    report['binary_sha256'] = hashlib.sha256(BINARY.read_bytes()).hexdigest()
    report['settle_ms'], report['deadline_ms'] = 300, 2000

    def observe(*blocks):
        return request('observe', blocks=list(blocks))

    def state(pos, name):
        return until(lambda: b if (b := observe(pos)['blocks'][0]['state']) and
                     b['value']['name'] == 'minecraft:' + name else None, 5)

    def prepare():
        command('tp ClimbingProbe .5 65 .5 0 0')
        assert 'error' not in request('prepare', position=[.5, 65., .5])

    def complete(identity, expected='accepted'):
        result = until(lambda: s if (s := request('status', id=identity))['status'] != 'pending' else None, 5)
        assert result['status'] == expected, result
        request('wait', ms=80)
        return result

    def sent(name, **args):
        mark, start = trace.mark(), time.monotonic()
        result = request('send_and_trust', **args)
        returned = time.monotonic()
        seconds = returned - start
        packet_ids = ({0x13} if legacy else {0x1d, 0x1e, 0x1f, 0x20}) if args['kind'] in ('move', 'look') else ({0x2d} if legacy else {0x3f}) if args['kind'] == 'place' else ({0x1b} if legacy else {0x28})
        def outbound():
            return [f for f in trace.since(mark) if f['phase'] == 'play' and f['direction'] == 'serverbound' and f['packet_id'] in packet_ids]
        frames = until(outbound, 2)
        # Admission may wait for the actor while chunks arrive. Measure from the
        # actual original action packet, rather than including that pre-send work.
        after_send = max(0., returned - frames[0]['observed_monotonic_seconds'])
        entry = dict(name=name, response=result, seconds=seconds, after_send_seconds=after_send,
                     frames=trace.since(mark))
        report['checks'].append(entry)
        assert result.get('dispatch', {}).get('status') == 'sent', entry
        assert after_send < .25, entry  # default quiet window alone is 300ms
        return entry, result['id']

    def delayed(name, packets, **args):
        entered, release = threading.Event(), threading.Event()
        def gate(frame):
            if frame['phase'] == 'play' and frame['direction'] == 'clientbound' and frame['packet_id'] in packets:
                entered.set()
                if not release.wait(5):
                    raise RuntimeError('bounded original-packet gate expired')
        trace.before_forward = gate
        try:
            entry, identity = sent(name, **args)
            assert entered.wait(2), entry
            # The returned call precedes even the first original processing/result packet.
            assert request('status', id=identity)['status'] == 'pending'
            entry['returned_before_original_reply'] = True
            return entry, identity
        finally:
            release.set()
            trace.before_forward = lambda frame: None

    command('fill -4 64 -4 4 64 8 minecraft:stone')
    command('fill -4 65 -4 4 70 8 minecraft:air')
    prepare()
    entry, looked = sent('look_full_send', kind='look', yaw=37., pitch=13.)
    assert request('status', id=looked)['status'] == 'pending'
    entry['result'] = complete(looked)
    entry['native_rotation'] = command('data get entity ClimbingProbe Rotation')
    assert '37.0f' in entry['native_rotation'] and '13.0f' in entry['native_rotation']

    # A correction after Accepted must also be discoverable by events_after alone.
    command('tp ClimbingProbe .5 65 .5 0 0')
    request('prepare_pose', position=[.5, 65., .5])
    events = request('events')
    assert any(e['action'] == looked and e['status']['status'] == 'corrected' for e in events['events']), events
    assert request('status', id=looked)['status'] == 'corrected'
    report['checks'].append(dict(name='late_teleport_after_accepted', events=events))

    controls = dict(forward=1, strafe=0, jump=False, sneak=False, sprint=False, yaw=0., pitch=0.)
    entry, moving = sent('finite_move_first_tick', kind='move', controls=controls, ticks=12)
    record = request('control_record')
    assert 0 < record['dispatched_ticks'] < 12, record
    assert record['status']['status'] == 'running', record
    entry['first_record'] = record
    entry['result'] = complete(moving)
    assert request('control_record')['dispatched_ticks'] >= 12
    entry['native_position'] = command('data get entity ClimbingProbe Pos')
    prepare()

    entry, held = sent('held_move_first_tick', kind='move', controls=controls)
    complete(held)
    assert request('control_record')['status']['status'] == 'running'
    assert request('release') == 'Released'
    entry['released_record'] = request('control_record')
    assert entry['released_record']['status']['status'] == 'stopped'
    prepare()

    selected = request('submit', kind='select', slot=0)
    complete(selected)
    replace = ('replaceitem entity ClimbingProbe weapon.mainhand ' if legacy else
               'item replace entity ClimbingProbe weapon.mainhand with ')
    command(replace + 'minecraft:gold_block 8')
    until(lambda: (s := observe()['player']['inventory'][36]) and isinstance(s['value'], dict)
          and s['value'].get('Item', {}).get('name') == 'minecraft:gold_block', 5)
    target = [2, 65, 0]
    command('setblock 2 65 0 minecraft:air')
    state(target, 'air')
    place = dict(kind='place', pos=[2, 64, 0], face='Up', cursor=[.17, 1., .83], target=target)
    entry, placed = delayed('place_full_send_before_reply', {0x0b} if legacy else {0x04}, **place)
    entry['result'] = complete(placed)
    entry['native_block'] = command('execute if block 2 65 0 minecraft:gold_block')
    assert 'Test passed' in entry['native_block']

    command('setblock 2 65 0 minecraft:diamond_block')
    state(target, 'diamond_block')
    entry, occupied = delayed('occupied_cell_later_corrected', {0x0b} if legacy else {0x04, 0x08}, **place)
    entry['result'] = complete(occupied, 'corrected')
    assert entry['result']['correction']['Block']['state']['value']['name'] == 'minecraft:diamond_block'
    assert any(e['action'] == occupied and e['status']['status'] == 'corrected' for e in request('events')['events'])

    command('setblock 2 65 0 minecraft:dirt')
    state(target, 'dirt')
    entry, broken = delayed('survival_break_start_before_ack', {0x07} if legacy else {0x04},
                            kind='break', pos=target, face='West')
    entry['result'] = complete(broken)
    entry['native_air'] = command('execute if block 2 65 0 minecraft:air')
    assert 'Test passed' in entry['native_air']

    # Keep the real START ACK pending beyond the absolute deadline. START is
    # already Sent, but the action must become Unknown and send one ABORT.
    command('setblock 2 65 0 minecraft:dirt')
    state(target, 'dirt')
    entered, release = threading.Event(), threading.Event()
    def gate(frame):
        if frame['phase'] == 'play' and frame['direction'] == 'clientbound' and frame['packet_id'] == (0x07 if legacy else 0x04):
            entered.set()
            if not release.wait(5): raise RuntimeError('deadline gate expired')
    trace.before_forward = gate
    mark = trace.mark()
    try:
        entry, unknown = sent('sent_but_missing_ack_deadline', kind='break', pos=target, face='West')
        assert entered.wait(2)
        entry['result'] = complete(unknown, 'unknown')
        digs = [bytes.fromhex(f['body_hex'])[0] for f in trace.since(mark)
                if f['phase'] == 'play' and f['direction'] == 'serverbound' and
                f['packet_id'] == (0x1b if legacy else 0x28)]
        assert digs == [0, 1], digs
        entry['dig_statuses'] = digs
    finally:
        release.set()
        trace.before_forward = lambda frame: None
    request('wait', ms=100)

    world = observe()['world']
    world['generation'] += 1
    for name, arguments in [
        ('k1_reach', dict(kind='break', pos=[30, 65, 0], face='West')),
        ('k1_coordinates', dict(place, pos=[33554432, 64, 0])),
        ('k2_world', dict(kind='look', yaw=1., pitch=0., world=world)),
        ('k2_slot', dict(kind='select', slot=9)),
    ]:
        mark, start = trace.mark(), time.monotonic()
        result = request('send_and_trust', **arguments)
        assert result.get('dispatch', {}).get('status') == 'not_sent', result
        assert result['dispatch']['outcome']['status'] == 'rejected', result
        assert time.monotonic() - start < .25
        assert not [f for f in trace.since(mark) if f['phase'] == 'play' and f['direction'] == 'serverbound'], trace.since(mark)
        report['checks'].append(dict(name=name, result=result))
        request('wait', ms=30)
    trace.expect_disconnect()
    request('disconnect')


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--accept-eula', action='store_true', required=True)
    p.add_argument('--version', choices=['1.16.1', '1.21.11'], required=True)
    p.add_argument('--binary', type=Path, required=True)
    p.add_argument('--jars', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    args = p.parse_args()
    BINARY = args.binary.resolve()
    os.environ['VOXRIG_BODY_TIMEOUT_MS'] = '2000'
    args.output.mkdir(parents=True, exist_ok=True)
    harness.ROOT = args.output.resolve() / 'runs'
    try:
        harness.run(args.version, BINARY, args.jars.resolve(), check=check)
    finally:
        if harness.ROOT.exists():
            for folder in harness.ROOT.iterdir():
                for name in ('report.json', 'packets.jsonl', 'probe.log', 'probe.stderr.log', 'server.log'):
                    if (folder / name).exists(): shutil.copyfile(folder / name, args.output / name)
