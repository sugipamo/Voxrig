#!/usr/bin/env python3
"""Qualify owned Elytra glide with the same frozen common Client on official servers."""
import argparse
import functools
import hashlib
import json
import re
import struct
import subprocess
from pathlib import Path
from run_climbing_control import REPO, run
from run_common_native import PacketTraceProxy, until


def check(version, command, request, trace, report, sdk):
    report['sdk'] = sdk
    modern = version == '1.21.11'
    position_packet = 0x1e if modern else 0x13
    # The actual packet ID is additionally observed from the unique native action
    # body, so version registry changes cannot be hidden by this expectation.
    start_action = 6 if modern else 8
    action_packet = 0x29 if modern else 0x1c

    def save(name, **facts):
        report['checks'].append(dict(name=name, **facts))

    def controls(**values):
        return dict(dict(forward=0, strafe=0, jump=False, sneak=False,
                         sprint=False, yaw=0., pitch=0.), **values)

    def admitted(result):
        assert result['admitted'], result
        return result['record']

    def raw(sequence):
        return until(lambda: frames[sequence-1] if len(frames := [f for f in trace.since(0)
            if f['direction'] == 'clientbound' and f['phase'] in ('configuration', 'play')]) >= sequence else None, 5)

    def equip(item='minecraft:elytra'):
        before = request('player')
        existing = before['inventory']['slots'][6]
        if item == 'minecraft:air' and existing and existing['source']['kind'] == 'received' and existing['value']['kind'] == 'empty':
            return existing  # Existing actual empty receipt; no fresh packet claimed.
        boundary = before['receive_sequence']
        command(('item replace entity ClimbingProbe armor.chest with ' if modern else
                 'replaceitem entity ClimbingProbe armor.chest ') + item)
        return until(lambda: s if (s := request('player')['inventory']['slots'][6]) and
            s['source']['kind'] == 'received' and s['source']['sequence'] > boundary else None, 5)

    def actual(value):
        return until(lambda: s if (s := request('player')['fall_flying']) and
                     s['source']['kind'] == 'received' and s['value'] == value else None, 5)

    def reset(height=78.):
        # Remove gear first to obtain an actual cleared flag; an owner stop does
        # not invent a nonexistent STOP_FALL_FLYING packet or claim physical rest.
        equip('minecraft:air')
        if request('player')['fall_flying']:
            actual(False)
        command('fill -4 65 -4 4 84 4 minecraft:air')
        command(f'tp ClimbingProbe .5 {height} .5 0 0')
        request('prepare', position=[.5, height, .5])
        equipment = equip()
        # Real play starts glide after an owned jump/fall tick. A fixture tp
        # invalidates ground provenance; it cannot turn a retained ground flag
        # into an airborne admission or a server acknowledgement.
        falling = admitted(request('mode_start', mode='survival'))
        frame = request('ticks', count=2)
        assert frame['frame']['on_ground'] == (height == 65.), frame
        admitted(request('mode_stop', mode='survival', session_id=falling['session_id']))
        return equipment

    def start(**keys):
        mark = trace.mark()
        owner = admitted(request('glide_start', mode='survival', controls=controls(**keys)))
        assert owner['kind'] == 'glide' and owner['frame'] is None, owner
        assert owner['glide']['accepted_sequence'] is None, owner
        running = until(lambda: r if (r := request('record')) and r['glide']['accepted_sequence'] is not None else None, 5)
        assert running['glide']['dispatched'], running
        flag = raw(running['glide']['accepted_sequence'])
        body = bytes.fromhex(flag['body_hex'])
        entity, offset = PacketTraceProxy.varint(body)
        assert entity == request('player')['entity_id'] and body[offset:offset+2] == b'\x00\x00' and body[offset+2] & 128, flag
        own = request('player')['entity_id']
        expected = bytes([own])
        # The fixtures use small actual login IDs; no invented native entity ID.
        assert 0 <= own < 128
        expected += bytes([start_action, 0])
        def starts():
            return [f for f in trace.since(mark) if f['direction'] == 'serverbound' and
                    f['phase'] == 'play' and f['packet_id'] == action_packet and f['body_hex'] == expected.hex()]
        until(lambda: starts(), 5)
        writes = starts(); assert len(writes) == 1, writes
        native = command('data get entity ClimbingProbe FallFlying')
        assert native.endswith('1b'), native
        save('actual_glide_start', initial=owner, confirmed=running,
             original_flag=flag, original_start=writes[0], native_flag=native)
        return owner, mark

    def inspect(name, record, mark):
        assert record['status']['status'] == 'running' and record['corrections'] == 0, record
        frame = record['frame']
        assert frame['gliding'] and frame['pose'] == 'FallFlying', frame
        assert abs(frame['bounds'][4]-frame['bounds'][1] - struct.unpack('>f', struct.pack('>f', .6))[0]) < 1e-9
        assert abs(frame['eye_position'][1]-frame['position'][1] - struct.unpack('>f', struct.pack('>f', .4))[0]) < 1e-9
        count = record['dispatched_ticks']
        def moves():
            return [f for f in trace.since(mark) if f['direction'] == 'serverbound' and
                    f['phase'] == 'play' and f['packet_id'] == position_packet]
        until(lambda: len(moves()) >= count, 5)
        flags = int(frame['on_ground']) | (int(frame['horizontal_collision']) << 1 if modern else 0)
        expected = struct.pack('>dddff', *frame['position'], record['controls']['yaw'], record['controls']['pitch']) + bytes([flags])
        wire = moves()[count-1]; assert wire['body_hex'] == expected.hex(), (wire, record)
        text = command('data get entity ClimbingProbe Pos'); match = re.search(r'\[([^]]+)\]', text); assert match, text
        native = [float(x.strip().removesuffix('d')) for x in match[1].split(',')]
        assert max(abs(a-b) for a,b in zip(native, frame['position'])) < .6, (native, record)
        save(name, record=record, original_movement=wire, native_position=native)

    def stopped():
        return until(lambda: r if (r := request('record')) and r['status']['status'] == 'stopped' else None, 8)

    def stable(record):
        mark = trace.mark(); assert request('wait', ms=150) == record
        assert not any(f['direction'] == 'serverbound' and f['phase'] == 'play' and
                       f['packet_id'] == position_packet for f in trace.since(mark))

    command('gamerule '+('minecraft:keep_inventory' if modern else 'keepInventory')+' true')
    command('gamerule '+('minecraft:mob_drops' if modern else 'doMobLoot')+' false')
    # Broken equipment and ordinary ground refuse before any start or movement.
    reset(65.)
    refused = request('glide_start', mode='survival', controls=controls())
    assert not refused['admitted'], refused
    save('ground_glide_start_refused', refusal=refused)
    reset()
    broken = equip('minecraft:elytra[minecraft:damage=431]' if modern else 'minecraft:elytra{Damage:431}')
    refused = request('glide_start', mode='survival', controls=controls()); assert not refused['admitted'], refused
    save('broken_actual_elytra_refused', equipment=broken, refusal=refused)

    reset(); owner, mark = start()
    inspect('neutral_glide', request('ticks', count=6), mark)
    admitted(request('mode_keys', mode='survival', session_id=owner['session_id'], controls=controls(yaw=37., pitch=-30.)))
    inspect('actual_glide_turn_and_lift', request('ticks', count=6), mark)
    admitted(request('mode_keys', mode='survival', session_id=owner['session_id'], controls=controls(pitch=90.)))
    landed = stopped(); assert landed['frame']['on_ground'], landed
    stable(landed); end = actual(False)
    save('controlled_descent_and_native_landing', record=landed, actual_end=end, original_end=raw(end['source']['sequence']))
    ground = admitted(request('mode_start', mode='survival'))
    assert ground['kind'] == 'ground' and ground['session_id'] != owner['session_id'], ground
    stale = request('mode_keys', mode='survival', session_id=owner['session_id'], controls=controls(forward=1)); assert not stale['admitted'], stale
    admitted(request('mode_stop', mode='survival', session_id=ground['session_id']))
    save('explicit_ground_after_actual_end_and_stale_id_refusal', owner=ground, stale_refusal=stale)

    reset(); command('fill -4 65 3 4 84 3 minecraft:stone')
    owner, mark = start()
    until(lambda: r if (r := request('record')) and r['frame'] and r['frame']['horizontal_collision'] else None, 6)
    wall = request('record'); inspect('actual_glide_wall', wall, mark)
    released = admitted(request('mode_stop', mode='survival', session_id=owner['session_id']))
    stable(released); save('owner_stop_releases_without_claiming_physical_glide_end', record=released, actual=request('player')['fall_flying'])

    reset(); owner, mark = start(); inspect('before_equipment_loss', request('ticks', count=3), mark)
    loss = equip('minecraft:air'); ended = stopped()
    assert ended['glide']['equipment_invalidated_sequence'] is not None or ended['glide']['ended_sequence'] is not None, ended
    stable(ended); actual(False); save('actual_equipment_loss_ends_owner', record=ended, equipment=loss)

    reset(); owner, mark = start(); request('ticks', count=3)
    command('tp ClimbingProbe 1.5 77.0 .5 0 0'); corrected = stopped()
    assert 'correction' in corrected['status']['reason'], corrected
    stable(corrected); save('actual_correction_discards_glide_origin', record=corrected, actual_pose=request('player')['received_pose'])

    if modern:
        reset(); owner, mark = start(); request('ticks', count=3)
        command('summon minecraft:pig .5 75 .5')
        command('ride ClimbingProbe mount @e[type=minecraft:pig,limit=1,sort=nearest]')
        mounted = until(lambda: v if (v := request('vehicle'))['relation'] and
                        v['relation']['value']['kind'] == 'mounted' else None, 5)
        ended = stopped(); stable(ended)
        assert ended['session_id'] == owner['session_id'] and ended['status']['status'] == 'stopped'
        stale = request('mode_keys', mode='survival', session_id=owner['session_id'], controls=controls())
        assert not stale['admitted'], stale
        save('actual_forced_mount_ends_glide_owner', record=ended, actual_mount=mounted, stale_refusal=stale)
        command('ride ClimbingProbe dismount')
        until(lambda: v if (v := request('vehicle'))['relation'] and
              v['relation']['value']['kind'] == 'unmounted' else None, 5)
        command('kill @e[type=minecraft:pig]')

    reset(); owner, mark = start(); request('ticks', count=3); world = request('player')['session']
    command('kill ClimbingProbe'); dead = stopped(); request('respawn')
    until(lambda: request('player')['session'] != world, 10)
    command('tp ClimbingProbe .5 65 .5 0 0'); request('prepare', position=[.5,65.,.5])
    stale = request('mode_keys', mode='survival', session_id=owner['session_id'], controls=controls()); assert not stale['admitted'], stale
    save('actual_death_respawn_rejects_old_glide_owner', record=dead, old_world=world, new_world=request('player')['session'], stale_refusal=stale)
    trace.expect_disconnect(); request('disconnect')


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary', type=Path, required=True); p.add_argument('--jars', type=Path, required=True)
    p.add_argument('--version', choices=['1.16.1','1.21.11']); p.add_argument('--compiled-sdk-revision', required=True)
    a = p.parse_args(); binary = a.binary.resolve()
    revision = subprocess.check_output(['git','rev-parse','HEAD'], cwd=REPO, text=True).strip()
    compiled = subprocess.check_output(['git','rev-parse',a.compiled_sdk_revision], cwd=REPO, text=True).strip()
    paths = ['src','examples','Cargo.toml','Cargo.lock','data']
    assert not subprocess.check_output(['git','diff',compiled,'HEAD','--',*paths], cwd=REPO)
    assert not subprocess.check_output(['git','diff','HEAD','--',*paths], cwd=REPO)
    artifacts = ['Cargo.lock','scripts/run_glide_live.py','scripts/run_glide_oracle.py',
        'scripts/movement_oracle/glide_scenarios.json','scripts/movement_oracle/run.py',
        'scripts/movement_oracle/java_1_16_1/MovementOracle.java','scripts/movement_oracle/java_1_21_11/MovementOracle.java',
        'scripts/run_climbing_control.py','scripts/run_common_native.py','examples/climbing_control_probe.rs','data/client_api/glide_oracle.json.gz']
    sdk = dict(source_revision=revision, binary_build_revision=compiled,
        binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),
        artifacts={f:hashlib.sha256((REPO/f).read_bytes()).hexdigest() for f in artifacts})
    for version in [a.version] if a.version else ['1.16.1','1.21.11']:
        assert hashlib.sha256(binary.read_bytes()).hexdigest() == sdk['binary_sha256']
        run(version, binary, a.jars.resolve(), check=functools.partial(check,sdk=sdk))
        assert hashlib.sha256(binary.read_bytes()).hexdigest() == sdk['binary_sha256']
if __name__ == '__main__':
    main()
