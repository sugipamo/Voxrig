#!/usr/bin/env python3
"""All HeadlessBody actions on isolated SHA-1 verified official native servers.

Uses the existing original-byte trace proxy and independent RCON oracle.
"""
import argparse
import hashlib
import re
import sys
import time
from pathlib import Path

sys.dont_write_bytecode = True

import run_climbing_control as harness
from run_common_native import until


def check(version, command, request, trace, report):
    legacy = version == '1.16.1'
    report['binary_sha256'] = hashlib.sha256(BINARY.read_bytes()).hexdigest()

    def observe(*blocks):
        return request('observe', blocks=list(blocks))

    def complete(identity):
        assert 'error' not in identity, identity
        result = until(lambda: s if (s := request('status', id=identity))['status'] != 'pending' else None, 12)
        request('wait', ms=80)
        return result

    def action(name, expected='accepted', **arguments):
        mark = trace.mark()
        started = time.monotonic()
        identity = request('submit', **arguments)
        result = complete(identity)
        assert result['status'] == expected, (name, result)
        record = dict(name=name, result=result, seconds=time.monotonic()-started,
                      frames=trace.since(mark))
        report['checks'].append(record)
        return record

    def position():
        text = command('data get entity ClimbingProbe Pos')
        return [float(v.strip().rstrip('df')) for v in re.search(r'\[([^]]+)\]', text)[1].split(',')]

    def prepare():
        command('tp ClimbingProbe .5 65 .5 0 0')
        assert 'error' not in request('prepare', position=[.5, 65., .5])

    def state(pos, name):
        return until(lambda: (b if (b := observe(pos)['blocks'][0]['state']) and b['value']['name'] == 'minecraft:' + name else None), 5)

    def held(count):
        p = observe()['player']
        item = (p['inventory'][36] or {}).get('value', {}).get('Item', {})
        return p if item.get('name') == 'minecraft:bread' and item.get('count') == count else None

    prepare()
    report['capabilities'] = request('capabilities')
    assert len(report['capabilities']['actions']) == 7
    looked = action('look', kind='look', yaw=37., pitch=13.)
    looked['native_rotation'] = command('data get entity ClimbingProbe Rotation')
    assert '37.0f' in looked['native_rotation'] and '13.0f' in looked['native_rotation']
    action('select_hotbar', kind='select', slot=0)
    action('chat', kind='chat', text='legacy body action live check')
    action('command', kind='command', text='say body-command-live')

    command('difficulty normal')
    replace = 'replaceitem entity ClimbingProbe weapon.mainhand ' if legacy else 'item replace entity ClimbingProbe weapon.mainhand with '
    command(replace + 'minecraft:bread 3')
    command('effect give ClimbingProbe minecraft:hunger 30 255 true')
    before = until(lambda: p if (p := held(3)) and p['health']['value']['food'] <= 12 else None, 12)
    command('effect clear ClimbingProbe minecraft:hunger')
    action('use_item_eat', kind='use')
    after = until(lambda: held(2), 8)
    assert after['health']['value']['food'] > before['health']['value']['food']
    request('release_use_item')
    mark = trace.mark()
    refused = complete(request('body_use_item_expect', slot=1, item='minecraft:bread'))
    assert refused['status'] == 'rejected', refused
    assert not any(f['direction'] == 'serverbound' and f['phase'] == 'play' and f['packet_id'] == (0x2e if legacy else 0x40) for f in trace.since(mark))
    report['checks'].append(dict(name='expected_item_identity_kept', result=refused))
    used = complete(request('body_use_item_expect', slot=0, item='minecraft:bread'))
    assert used['status'] == 'accepted', used
    until(lambda: held(1), 8)
    request('release_use_item')
    native_food = command('data get entity ClimbingProbe Inventory')
    assert 'minecraft:bread' in native_food
    report['checks'].append(dict(name='use_item_expect_eat', result=used, native_inventory=native_food))

    command('fill -4 65 -4 4 70 12 minecraft:air')
    command('fill -4 64 -4 4 64 12 minecraft:stone')
    command('setblock 0 65 2 minecraft:oak_slab[type=bottom]')
    command('setblock 0 65 3 minecraft:oak_stairs[facing=south,half=bottom,shape=straight]')
    command('fill 0 65 4 0 65 8 minecraft:stone')
    command('summon minecraft:skeleton 3 65 3 {Tags:["BodyContact"],NoAI:1b,NoGravity:1b,Invulnerable:1b}')
    until(lambda: any(e['motion']['entity']['type_name'] == 'minecraft:skeleton' for e in request('entities')['entities']), 8)
    prepare()
    controls = dict(forward=1, strafe=0, jump=False, sneak=False, sprint=False, yaw=0., pitch=0.)
    walked = action('walk_up_steps_best_effort', kind='move', controls=controls, ticks=25)
    walked['native_position'] = position()
    assert walked['native_position'][2] > 3.5 and walked['native_position'][1] > 65.5, walked
    record = request('control_record')
    walked['contact_diagnostic'] = request('contact_diagnostic', session_id=record['session_id'])
    assert walked['contact_diagnostic'], walked
    controls['yaw'] = 180.
    returned = action('walk_down_steps', kind='move', controls=controls, ticks=25)
    returned['native_position'] = position()
    assert returned['native_position'][2] < 2., returned
    command('kill @e[tag=BodyContact]')
    command('fill -4 65 -4 4 70 12 minecraft:air')
    prepare()
    action('select_for_dig', kind='select', slot=0)
    command('setblock 2 65 0 minecraft:dirt')
    state([2,65,0], 'dirt')
    broken = action('survival_break_ack', kind='break', pos=[2,65,0], face='West')
    broken['native_block'] = command('execute if block 2 65 0 minecraft:air')
    assert 'Test passed' in broken['native_block']
    assert any(f['direction'] == 'clientbound' and f['phase'] == 'play' and f['packet_id'] == (0x07 if legacy else 0x04) for f in broken['frames'])

    command('setblock 2 65 0 minecraft:dirt')
    state([2,65,0], 'dirt')
    identity = request('submit', kind='break', pos=[2,65,0], face='West')
    request('wait', ms=100)
    command('setblock 2 65 0 minecraft:diamond_block')
    rolled = complete(identity)
    assert rolled['status'] == 'corrected', rolled
    assert rolled['correction']['Block']['state']['value']['name'] == 'minecraft:diamond_block', rolled
    report['checks'].append(dict(name='block_replacement_correction', result=rolled))

    identity = request('submit', kind='look', yaw=81., pitch=4.)
    request('wait', ms=80)
    command('tp ClimbingProbe 1.5 65 .5 12 3')
    corrected = complete(identity)
    assert corrected['status'] == 'corrected' and 'Pose' in corrected['correction'], corrected
    report['checks'].append(dict(name='pose_correction', result=corrected))
    report['events'] = request('events')
    assert not report['events']['gap']
    trace.expect_disconnect()
    request('disconnect')


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--accept-eula', action='store_true', required=True)
    p.add_argument('--binary', type=Path, required=True)
    p.add_argument('--jars', type=Path, required=True)
    p.add_argument('--version', choices=['1.16.1', '1.21.11'], required=True)
    p.add_argument('--output', type=Path, required=True)
    args = p.parse_args()
    BINARY = args.binary.resolve()
    harness.ROOT = args.output.resolve()
    harness.run(args.version, BINARY, args.jars.resolve(), check=check)
