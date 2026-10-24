#!/usr/bin/env python3
"""Acceptance API checks on a disposable, SHA-1 verified official 1.21.11 server.

Uses the established byte-for-byte trace proxy and independent RCON oracle.
Requires --accept-eula. Only a temporary server directory is mutated.
"""
import argparse
import hashlib
import json
import re
import shutil
import struct
import sys
import tempfile
import threading
import time
from pathlib import Path
sys.dont_write_bytecode = True

import run_climbing_control as harness
from run_common_native import until


def check(version, command, request, trace, report):
    assert version == '1.21.11'

    def observe(*blocks):
        return request('observe', blocks=list(blocks))

    def mode(wanted):
        command('gamemode ' + wanted + ' ClimbingProbe')
        until(lambda: observe()['player']['game_mode'] == wanted, 5)

    def complete(identity):
        def poll():
            status = request('status', id=identity)
            assert 'error' not in status, status
            return status if status['status'] != 'pending' else None
        result = until(poll, 12)
        # Return after owned cleanup, so the next action can acquire control.
        request('wait', ms=60)
        return result

    def action(name, expected='accepted', **arguments):
        boundary = trace.mark()
        start = time.monotonic()
        identity = request('submit', **arguments)
        assert 'error' not in identity, identity
        result = complete(identity)
        assert result['status'] == expected, (name, result)
        entry = dict(name=name, id=identity, result=result,
                     seconds=time.monotonic()-start, frames=trace.since(boundary))
        report['checks'].append(entry)
        return entry

    command('tp ClimbingProbe 0.5 65 0.5 0 0')
    request('prepare', position=[.5,65.,.5])
    report['capabilities'] = request('capabilities')
    assert report['capabilities']['observation']
    looked = action('look_without_pose_echo', kind='look', yaw=37., pitch=13.)
    assert not any(f['direction']=='clientbound' and f['phase']=='play' and f['packet_id']==0x46 for f in looked['frames'])
    native = command('data get entity ClimbingProbe Rotation')
    assert '37.0f' in native and '13.0f' in native, native
    report['checks'][-1]['native_rotation'] = native
    selected = action('select_without_slot_echo', kind='select', slot=0)
    assert not any(f['direction']=='clientbound' and f['phase']=='play' and f['packet_id']==0x67 for f in selected['frames'])

    controls = dict(forward=1, strafe=0, jump=False, sneak=False, sprint=False, yaw=0., pitch=0.)
    move = action('move_ten_ticks_without_pose_echo', kind='move', controls=controls, ticks=10)
    assert not any(f['direction']=='clientbound' and f['phase']=='play' and f['packet_id']==0x46 for f in move['frames'])
    native = command('data get entity ClimbingProbe Pos')
    coords = [float(v.strip().rstrip('df')) for v in re.search(r'\[([^]]+)\]', native)[1].split(',')]
    assert coords[2] > .8, native
    move['native_position'] = native

    command('tp ClimbingProbe 0.5 65 0.5 0 0')
    request('prepare', position=[.5,65.,.5])
    command('item replace entity ClimbingProbe weapon.mainhand with minecraft:gold_block 4')
    until(lambda: (s := observe()['player']['inventory'][36]) and isinstance(s['value'], dict) and s['value'].get('Item',{}).get('name') == 'minecraft:gold_block', 5)
    command('setblock 2 65 0 minecraft:air')
    until(lambda: observe([2,65,0])['blocks'][0]['state']['value']['name'] == 'minecraft:air', 5)
    placed = action('arbitrary_gold_block_exact_hit', kind='place', pos=[2,64,0], face='Up', cursor=[.17,1.,.83], target=[2,65,0])
    placed['native_block'] = command('execute if block 2 65 0 minecraft:gold_block')
    assert 'Test passed' in placed['native_block'], placed
    uses = [f for f in placed['frames'] if f['direction']=='serverbound' and f['phase']=='play' and f['packet_id']==0x3f]
    assert len(uses) == 1, uses
    raw = bytes.fromhex(uses[0]['body_hex'])
    assert raw[0] == 0 and raw[9] == 1, raw.hex()  # Main hand and Up face.
    assert raw[10:22] == struct.pack('>fff', .17, 1., .83), raw.hex()
    assert any(f['direction']=='clientbound' and f['phase']=='play' and f['packet_id']==0x04 for f in placed['frames']), placed

    # The held gold block cannot replace an occupied diamond cell. Send the real
    # hit anyway and require the server's diamond rollback, not a local reach check.
    command('setblock 2 65 0 minecraft:diamond_block')
    until(lambda: observe([2,65,0])['blocks'][0]['state']['value']['name'] == 'minecraft:diamond_block', 5)
    identity = request('submit', kind='place', pos=[2,64,0], face='Up', cursor=[.2,1.,.8], target=[2,65,0])
    corrected = complete(identity)
    assert corrected['status'] in ('corrected','rejected'), corrected
    if corrected['status'] == 'corrected':
        assert corrected['correction']['Block']['state']['value']['name'] == 'minecraft:diamond_block', corrected
    report['checks'].append(dict(name='occupied_placement_rollback', id=identity, result=corrected))

    command('item replace entity ClimbingProbe weapon.mainhand with minecraft:diamond_pickaxe')
    until(lambda: (s := observe()['player']['inventory'][36]) and isinstance(s['value'], dict) and s['value'].get('Item',{}).get('name') == 'minecraft:diamond_pickaxe', 5)
    broken = action('survival_dig_continuation', kind='break', pos=[2,65,0], face='West')
    broken['native_block'] = command('execute if block 2 65 0 minecraft:air')
    assert 'Test passed' in broken['native_block']
    digs = [f for f in broken['frames'] if f['direction']=='serverbound' and f['phase']=='play' and f['packet_id']==0x28]
    assert len(digs) == 2, digs

    mode('creative')
    command('setblock 2 65 0 minecraft:stone')
    until(lambda: observe([2,65,0])['blocks'][0]['state']['value']['name'] == 'minecraft:stone', 5)
    broken = action('creative_instant_break', kind='break', pos=[2,65,0], face='West')
    broken['native_block'] = command('execute if block 2 65 0 minecraft:air')
    assert 'Test passed' in broken['native_block']
    action('chat_without_delivery_receipt', kind='chat', text='body acceptance live check')
    action('command_without_success_receipt', kind='command', text='list')
    action('use_item', kind='use')

    # A real server teleport within the look settling window must report the
    # received pose, even though no normal look ACK exists.
    identity = request('submit', kind='look', yaw=81., pitch=4.)
    request('wait', ms=80)
    command('tp ClimbingProbe 1.5 65 0.5 12 3')
    corrected = complete(identity)
    assert corrected['status'] == 'corrected', corrected
    assert corrected['snapshot']['player']['position']['value'] == [1.5,65.,.5], corrected
    report['checks'].append(dict(name='teleport_rollback', result=corrected))
    request('prepare', position=[1.5,65.,.5])

    identity = request('submit', kind='move', controls=controls)
    until(lambda: request('status', id=identity)['status'] == 'accepted', 5)
    overlap = request('submit', kind='look', yaw=1., pitch=0.)
    assert request('status', id=overlap)['status'] == 'rejected'
    assert request('release') == 'Released'
    assert request('status', id=identity)['status'] == 'accepted'
    report['checks'].append(dict(name='held_input_release', id=identity, result='accepted then released'))
    identity = request('submit', kind='look', yaw=10., pitch=0.)
    request('cancel', id=identity)
    assert complete(identity)['status'] == 'cancelled'
    report['checks'].append(dict(name='cancel', id=identity))
    action('invalid_slot_rejected_before_send', expected='rejected', kind='select', slot=9)

    # Delay the original server ACK at the proxy, without changing packet bytes.
    # The server really breaks the block, but lack of delivered ACK must terminate
    # as Unknown. Releasing the late ACK must never rewrite that terminal result.
    command('setblock 3 65 0 minecraft:stone')
    until(lambda: observe([3,65,0])['blocks'][0]['state']['value']['name'] == 'minecraft:stone', 5)
    ack_gate = threading.Event()
    ack_seen = threading.Event()
    def delay_ack(record):
        if record['direction']=='clientbound' and record['phase']=='play' and record['packet_id']==0x04:
            ack_seen.set()
            if not ack_gate.wait(12):
                raise TimeoutError('fixture ACK gate was not released')
    trace.before_forward = delay_ack
    started = time.monotonic()
    try:
        missing_ack_id = request('submit', kind='break', pos=[3,65,0], face='West')
        missing_ack = complete(missing_ack_id)
        assert ack_seen.is_set(), 'no original ACK reached the timing fixture'
        assert missing_ack['status'] == 'unknown', missing_ack
        assert time.monotonic()-started < 11.5
        native = command('execute if block 3 65 0 minecraft:air')
        assert 'Test passed' in native, native
        report['checks'].append(dict(name='missing_ack_bounded_unknown', id=missing_ack_id,
                                     result=missing_ack, seconds=time.monotonic()-started, native_block=native))
    finally:
        ack_gate.set()
        trace.before_forward = lambda record: None
    request('wait', ms=100)
    assert request('status', id=missing_ack_id)['status'] == 'unknown'
    report['events'] = request('events')
    assert not report['events']['gap']
    observation = observe([2,65,0], [100000,65,100000])
    assert observation['blocks'][-1]['state'] is None, observation
    assert observation['player']['health']['source'].get('Received'), observation
    report['observation'] = observation
    trace.expect_disconnect()
    request('disconnect')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--accept-eula', action='store_true', required=True)
    parser.add_argument('--server-jar', type=Path, required=True)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix='voxrig-body-') as temporary:
        root = Path(temporary)
        jars = root/'jars'
        jars.mkdir()
        shutil.copyfile(args.server_jar, jars/'1.21.11-server.jar')
        harness.ROOT = root/'runs'
        try:
            harness.run('1.21.11', args.binary.resolve(), jars, check=check)
        finally:
            if harness.ROOT.exists():
                for folder in harness.ROOT.iterdir():
                    for name in ('report.json','packets.jsonl','probe.log','probe.stderr.log','server.log'):
                        if (folder/name).exists():
                            shutil.copyfile(folder/name, args.output/name)
    (args.output/'inputs.json').write_text(json.dumps(dict(binary_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest()), indent=2)+'\n')


if __name__ == '__main__':
    main()
