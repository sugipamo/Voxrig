#!/usr/bin/env python3
"""Step 3: explicit placement/activation during one moving owner, official servers."""
import argparse
import hashlib
import re
import struct
import sys
sys.dont_write_bytecode = True
from pathlib import Path
from run_climbing_control import run
from run_common_native import until


def check(version, command, request, trace, report):
    modern = version == '1.21.11'
    report['binary_sha256'] = hashlib.sha256(BINARY.read_bytes()).hexdigest()
    def observe(*blocks):
        return request('observe', blocks=list(blocks))
    def native_position():
        raw = command('data get entity ClimbingProbe Pos')
        return [float(s.strip().rstrip('df')) for s in re.search(r'\[([^]]+)\]', raw)[1].split(',')]
    def known(pos, name):
        return until(lambda: (v := observe(pos)['blocks'][0]['state']) and v['value']['name'] == 'minecraft:' + name and v, 6)
    def prepare(pos=(.5, 65., .2)):
        command('tp ClimbingProbe %s %s %s 180 0' % tuple(pos))
        result = request('prepare', position=list(pos))
        assert 'error' not in result, result
        request('wait', ms=1400)
    def held(name, count=32):
        command(('item replace entity ClimbingProbe hotbar.0 with ' if modern else
                 'replaceitem entity ClimbingProbe hotbar.0 ') + 'minecraft:' + name + ' ' + str(count))
        result = request('select', slot=0)
        assert 'error' not in result, result
        until(lambda: isinstance((s := observe()['player']['inventory'][36]), dict)
              and isinstance(s['value'], dict) and s['value'].get('Item', {}).get('name') == 'minecraft:' + name, 6)
    def complete(identity):
        assert 'error' not in identity, identity
        def poll():
            result = request('status', id=identity)
            assert 'error' not in result, result
            return result if result['status'] != 'pending' else None
        result = until(poll, 12)
        request('wait', ms=60)
        return result
    def keys(**changes):
        control = dict(forward=1, strafe=0, jump=False, sneak=True, sprint=False, yaw=0., pitch=0.)
        control.update(changes)
        result = request('control_set', controls=control)
        assert 'error' not in result, result
        return result
    def start():
        result = request('best_effort_start')
        assert 'error' not in result, result
        return result
    def stop(owner):
        result = request('control_stop')
        assert result['session_id'] == owner['session_id'], (owner, result)
        return result
    def placement(pos, face, target, helper=False, expected='accepted'):
        arguments = dict(kind='place', pos=pos, face=face, cursor=[.17, .6, 1.] if face == 'South' else [.17, 1., .83], target=target)
        before_owner = request('control_record')
        before = native_position()
        mark = trace.mark()
        identity = request('place_cube' if helper else 'submit', **arguments)
        result = complete(identity)
        assert result['status'] == expected, result
        frames = trace.since(mark)
        packets = [p for p in frames if p['direction'] == 'serverbound' and p['phase'] == 'play'
                   and p['packet_id'] == (0x3f if modern else 0x2d)]
        assert len(packets) == 1, packets
        raw = bytes.fromhex(packets[0]['body_hex'])
        assert raw[0] == 0 and raw[9] == (3 if face == 'South' else 1), raw.hex()
        assert raw[10:22] == struct.pack('>fff', *arguments['cursor']), raw.hex()
        after_owner = request('control_record')
        assert after_owner['session_id'] == before_owner['session_id'] and after_owner['status']['status'] == 'running', after_owner
        assert after_owner['dispatched_ticks'] > before_owner['dispatched_ticks'], (before_owner, after_owner)
        warnings = request('placement_warnings', **arguments)
        assert isinstance(warnings, list) and warnings, warnings
        entry = dict(name='explicit_' + face, target=target, result=result, frames=frames,
                     before=before, after=native_position(), owner=after_owner, warnings=warnings)
        if expected == 'accepted':
            known(target, 'gold_block')
            assert 'Test passed' in command('execute if block %s %s %s minecraft:gold_block' % tuple(target))
        report['checks'].append(entry)
        return entry

    command('fill -3 65 -3 3 75 12 minecraft:air')
    command('fill -2 62 1 2 64 12 minecraft:air')
    command('setblock 0 64 0 minecraft:stone')
    prepare()
    held('gold_block')
    warning = request('placement_warnings', kind='place', pos=[0,64,0], face='South', cursor=[.17,.6,1.], target=[0,64,1])
    assert isinstance(warning, list) and warning, warning
    strict = request('strict_place', pos=[0,64,0])
    assert 'error' in strict, strict
    report['checks'].append(dict(name='optional_diagnostics_are_warnings', warnings=warning, strict=strict))
    owner = start()
    keys()
    before = native_position()
    for z in range(1, 6):
        until(lambda: observe()['player']['position']['value'][2] > z - 3.8, 8)
        placement([0,64,z-1], 'South', [0,64,z], helper=True)
    until(lambda: observe()['player']['position']['value'][2] > 3.5, 10)
    stopped = stop(owner)
    after = native_position()
    assert after[2] > before[2] + 3. and abs(after[1] - 65.) < .02, (before, after)
    report['checks'].append(dict(name='five_block_bridge_same_walking_owner', before=before, after=after, owner=stopped))

    # Walking/jumping pillar: active horizontal input, explicit upward hit after
    # the feet clear the result cube. Repeat with a fresh position for determinism.
    command('fill -2 64 -2 2 64 2 minecraft:stone')
    for trial in range(2):
        command('setblock 0 65 0 minecraft:air')
        prepare((.5,65.,.05))
        known([0,65,0], 'air')
        owner = start()
        keys(jump=True)
        until(lambda: observe()['player']['position']['value'][1] > 66.02, 4)
        placed = placement([0,64,0], 'Up', [0,65,0], helper=trial == 0)
        assert any(f['direction']=='serverbound' and f['phase']=='play' and f['packet_id'] in
                   ((0x1d,0x1e) if modern else (0x12,0x13)) for f in placed['frames']), placed
        stopped = stop(owner)
        placed['name'] = 'pillar_while_walking_airborne_' + str(trial)
        placed['stopped'] = stopped

    # Off-axis opening while the same owner keeps walking on a broad floor.
    command('fill -4 64 -4 8 64 12 minecraft:stone')
    command('fill -3 65 -3 3 69 8 minecraft:air')
    command(('item replace entity ClimbingProbe hotbar.0 with minecraft:air' if modern else
             'replaceitem entity ClimbingProbe hotbar.0 minecraft:air'))
    until(lambda: observe()['player']['inventory'][36]['value'] == 'Empty', 6)
    for block in ['crafting_table', 'furnace', 'chest']:
        command('setblock 2 65 1 minecraft:' + block)
        prepare((.5,65.,.2))
        known([2,65,1], block)
        strict = request('strict_open', pos=[2,65,1])
        assert 'error' in strict, strict
        owner = start()
        keys(sneak=False)
        before = native_position()
        mark = trace.mark()
        identity = request('submit', kind='activate', pos=[2,65,1], face='East', cursor=[1.,.25,.75])
        result = complete(identity)
        assert result['status'] == 'accepted', result
        screen = until(lambda: (s if s.get('screen') else None) if (s := request('screen')) else None, 6)
        after_owner = request('control_record')
        after = native_position()
        assert after_owner['session_id'] == owner['session_id'] and after_owner['status']['status'] == 'running', after_owner
        assert after[2] > before[2] + .1, (before, after)
        frames = trace.since(mark)
        uses = [p for p in frames if p['direction']=='serverbound' and p['phase']=='play' and p['packet_id']==(0x3f if modern else 0x2d)]
        assert len(uses) == 1, uses
        raw = bytes.fromhex(uses[0]['body_hex'])
        assert raw[9] == 5 and raw[10:22] == struct.pack('>fff',1.,.25,.75), raw.hex()
        stop(owner)
        closed = request('close_screen')
        assert 'error' not in closed, closed
        report['checks'].append(dict(name='off_axis_moving_' + block, warnings=[strict['error']], result=result,
                                     screen=screen, owner=after_owner, before=before, after=after, frames=frames))
        command('setblock 2 65 1 minecraft:air')
        request('wait', ms=150)

    # Beyond the old client pick distance, within the official server limit.
    held('gold_block')
    prepare((.5,65.,.2))
    if modern:
        command('attribute ClimbingProbe minecraft:block_interaction_range base set 4.5')
        command('attribute ClimbingProbe minecraft:scale base set 1.0')
    owner = start()
    keys(forward=0)
    request('wait', ms=200)
    command('setblock 5 64 0 minecraft:stone')
    command('setblock 5 65 0 minecraft:air')
    known([5,65,0], 'air')
    far = placement([5,64,0], 'Up', [5,65,0])
    far['name'] = 'beyond_fixed_client_reach_accepted'
    # Occupied destination: actual block rollback, not a client obstruction veto.
    command('setblock 2 65 0 minecraft:diamond_block')
    known([2,65,0], 'diamond_block')
    corrected = placement([2,64,0], 'Up', [2,65,0], expected='corrected')
    corrected['name'] = 'server_rollback_while_owner_keeps_running'
    assert corrected['result']['correction']['Block']['state']['value']['name'] == 'minecraft:diamond_block'
    rejected = complete(request('submit', kind='place', pos=[40,64,0], face='Up', cursor=[.5,1.,.5], target=[40,65,0]))
    assert rejected['status'] == 'rejected' and 'official server distance' in rejected['reason'], rejected
    report['checks'].append(dict(name='official_server_hard_distance_limit', result=rejected))
    stop(owner)
    trace.expect_disconnect()
    request('disconnect')


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--accept-eula', action='store_true', required=True)
    p.add_argument('--binary', type=Path, required=True)
    p.add_argument('--jars', type=Path, required=True)
    p.add_argument('--version', choices=['1.16.1','1.21.11'], required=True)
    args = p.parse_args()
    BINARY = args.binary.resolve()
    run(args.version, BINARY, args.jars.resolve(), check=check)
