#!/usr/bin/env python3
"""Domain admission on local SHA-1 checked official servers; original bytes only.

A proxy delays a real inventory reply while independent client packets continue.
No packet or client cache is edited. RCON independently verifies server effects.
"""
import argparse
import hashlib
import json
import os
import re
import shutil
import threading
import time
from pathlib import Path

import run_climbing_control as harness
from run_common_native import until


def check(version, command, request, trace, report):
    report['binary_sha256'] = hashlib.sha256(BINARY.read_bytes()).hexdigest()
    keys = dict(forward=1, strafe=0, jump=False, sneak=False, sprint=False, yaw=0., pitch=0.)

    def complete(identity):
        def poll():
            value = request('status', id=identity)
            assert 'error' not in value, value
            return value if value['status'] != 'pending' else None
        result = until(poll, 6)
        request('wait', ms=80)
        return result

    def action(name, expect=('accepted',), **args):
        mark, started = trace.mark(), time.monotonic()
        identity = request('submit', **args)
        assert 'error' not in identity, identity
        result = complete(identity)
        if result['status'] not in expect:
            report['failed_control'] = request('control_record')
            raise AssertionError((name, result, report['failed_control']))
        entry = dict(name=name, result=result, seconds=time.monotonic()-started,
                     frames=trace.since(mark), id=identity)
        report['checks'].append(entry)
        return entry

    command('fill -5 64 -5 8 64 8 minecraft:stone')
    command('fill -4 65 -4 7 70 7 minecraft:air')
    command('gamemode survival ClimbingProbe')
    command('tp ClimbingProbe 0.5 65 0.5 0 0')
    request('prepare', position=[.5,65.,.5])
    until(lambda: request('player')['game_mode'] == 'survival')
    action('initial_selection', kind='select', slot=0)
    prefix = 'replaceitem entity' if version == '1.16.1' else 'item replace entity'
    suffix = ' minecraft:stone 8' if version == '1.16.1' else ' with minecraft:stone 8'
    tool = ' minecraft:diamond_shovel' if version == '1.16.1' else ' with minecraft:diamond_shovel'
    command(prefix + ' ClimbingProbe weapon.mainhand' + tool)
    command(prefix + ' ClimbingProbe inventory.0' + suffix)
    until(lambda: (s := request('player')['inventory']['slots'][9]) and
          isinstance(s['value'], dict) and 'item' in s['value'])
    record = request('click', slot=9)
    assert 'error' not in record, record
    accepted = until(lambda: (s if s['status'] != 'pending' else None)
                     if (s := request('click_status')) else None, 4)
    assert accepted['status'] == 'accepted', accepted
    # Return the cursor by a second explicit intent; never automatic replay.
    record = request('click', slot=9)
    assert 'error' not in record, record
    until(lambda: request('click_status')['status'] == 'accepted', 4)
    report['checks'].append(dict(name='fresh_exact_pickup_receipts', result=accepted))

    command('setblock 2 65 0 minecraft:chest')
    command('data merge block 2 65 0 {Items:[{Slot:0b,id:"minecraft:stone",Count:8b,count:8}]}')
    command('setblock 0 65 2 minecraft:dirt')
    action('open_real_container', kind='activate', pos=[2,65,0], face='West', cursor=[0.,.5,.5])
    opening = until(lambda: (s if s.get('screen') and s['screen'].get('slots') and
                            s['screen']['slots'][0] else None)
                    if (s := request('screen')) else None)
    report['opening'] = opening
    entered, release = threading.Event(), threading.Event()
    incoming = {0x11,0x14,0x16} if version == '1.16.1' else {0x12,0x14,0x5e,0x66}
    def delay(frame):
        if frame['phase'] == 'play' and frame['direction'] == 'clientbound' and frame['packet_id'] in incoming:
            entered.set()
            if not release.wait(20):
                raise RuntimeError('bounded inventory reply delay expired')
    trace.before_forward = delay
    mark = trace.mark()
    try:
        record = request('click', slot=0, container=True)
        assert 'error' not in record, record
        assert entered.wait(4), 'no real inventory reply arrived'
        assert request('click_status')['status'] == 'pending'
        conflict = request('click', slot=0, container=True)
        assert 'error' in conflict, conflict
        report['checks'].append(dict(name='same_screen_conflict_refused', result=conflict))
        # The same original server reply remains delayed throughout these sends.
        looked = action('pending_click_look', kind='look', yaw=30., pitch=5.)
        looked['native_rotation'] = command('data get entity ClimbingProbe Rotation')
        assert '30.0f' in looked['native_rotation'] and '5.0f' in looked['native_rotation'], looked
        dug = action('pending_click_dig', expect=('unknown',), kind='break', pos=[0,65,2], face='North')
        action('pending_click_move', kind='move', controls=keys, ticks=4)
        action('pending_click_chat', kind='chat', text='domain gate live check')
        action('pending_click_hotbar', kind='select', slot=1)
        assert request('click_status')['status'] == 'unknown'
        assert 'error' in request('click', slot=0, container=True)
        report['checks'].append(dict(name='receipt_deadline_unknown_without_replay', record=request('click_record')))
        digs = [f for f in dug['frames'] if f['phase']=='play' and f['direction']=='serverbound' and f['packet_id']==(0x1b if version=='1.16.1' else 0x28)]
        assert len(digs)==2, 'survival START/STOP must each be sent once'
        native = command('execute if block 0 65 2 minecraft:air')
        assert 'Test passed' in native, native
        dug['native_block'] = native
        native = command('data get entity ClimbingProbe Pos')
        coords = [float(v.strip().rstrip('df')) for v in re.search(r'\[([^]]+)\]',native)[1].split(',')]
        assert coords[2] > .65, native
        report['native_position'] = native
        report['native_rotation'] = command('data get entity ClimbingProbe Rotation')
        report['native_hotbar'] = command('data get entity ClimbingProbe SelectedItemSlot')
        assert '0.0f' in report['native_rotation'], report['native_rotation']
        assert re.search(r': 1\b', report['native_hotbar']), report['native_hotbar']
        # A later actual block rollback must remain visible after the deadline.
        command('setblock 0 65 2 minecraft:diamond_block')
        replace = 'replaceitem block' if version == '1.16.1' else 'item replace block'
        with_item = ' minecraft:diamond 1' if version == '1.16.1' else ' with minecraft:diamond 1'
        command(replace + ' 2 65 0 container.0' + with_item)
    finally:
        release.set()
        trace.before_forward = lambda frame: None
    late = until(lambda: (s if s['status']=='corrected' else None)
                 if (s := request('outcome', id=dug['id'])) else None, 5)
    assert request('status', id=dug['id'])['status'] == 'unknown'  # strict terminal API unchanged
    report['checks'].append(dict(name='late_dig_correction_after_unknown', result=late))
    report['delayed_click_outcome'] = until(lambda: (s if s['status']=='corrected' else None)
                                           if (s := request('click_status')) else None, 5)
    report['checks'].append(dict(name='late_inventory_correction_after_unknown', result=report['delayed_click_outcome']))
    report['pending_frames'] = trace.since(mark)
    clicks = [f for f in report['pending_frames'] if f['direction']=='serverbound' and f['phase']=='play' and
              f['packet_id']==(0x09 if version=='1.16.1' else 0x11)]
    assert len(clicks)==1, 'conflicting click or replay reached the writer' 
    trace.expect_disconnect()
    request('disconnect')


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--accept-eula', action='store_true', required=True)
    p.add_argument('--version', choices=['1.16.1','1.21.11'], required=True)
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
                for name in ('report.json','packets.jsonl','probe.log','probe.stderr.log','server.log'):
                    if (folder/name).exists(): shutil.copyfile(folder/name,args.output/name)
