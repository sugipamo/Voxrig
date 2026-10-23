#!/usr/bin/env python3
"""Verify relative armor-stand hits on a disposable official 1.21.11 server."""
import argparse
import hashlib
import json
from pathlib import Path
import shutil
import struct
import tempfile
import run_climbing_control as live
from run_common_native import until


def check(version, command, request, trace, report):
    assert version == '1.21.11'
    sources = ['Cargo.lock', 'Cargo.toml', 'examples/climbing_control_probe.rs',
               'src/client/operations.rs', 'src/versions/java_1_21_11/wire.rs',
               'src/versions/java_1_21_11/client/operations.rs',
               'scripts/run_entity_interact_at_live.py', 'scripts/run_climbing_control.py',
               'scripts/run_common_native.py']
    report['source_sha256'] = {p: hashlib.sha256((live.REPO / p).read_bytes()).hexdigest() for p in sources}
    report['binary_sha256'] = hashlib.sha256(BINARY.read_bytes()).hexdigest()
    command('gamerule minecraft:mob_drops false')
    command('kill @e[type=!minecraft:player]')
    command('tp ClimbingProbe 0.5 65 0.5 0 0')
    request('prepare', position=[.5, 65., .5])
    target_type = 'minecraft:armor_stand'
    selector = '@e[tag=AtTarget,limit=1]'

    def stands():
        return [e for e in request('entities')['entities'] if e['motion']['entity']['type_name'] == target_type]

    def frames(mark):
        return [f for f in trace.since(mark) if f['connection'] == 1 and f['direction'] == 'serverbound'
                and f['phase'] == 'play' and f['packet_id'] == 0x19]

    for mode in ('survival', 'creative'):
        command('gamemode ' + mode + ' ClimbingProbe')
        until(lambda: request('player')['game_mode'] == mode, 5)
        for offhand in (False, True):
            for height, wanted, retained in ((.1, 'iron_boots', 'iron_helmet'), (1.8, 'iron_helmet', 'iron_boots')):
                command('clear ClimbingProbe')
                command('summon minecraft:armor_stand 0.5 65 2 {Tags:["AtTarget"],NoGravity:1b,ShowArms:1b}')
                command('item replace entity ' + selector + ' armor.feet with minecraft:iron_boots')
                command('item replace entity ' + selector + ' armor.head with minecraft:iron_helmet')
                until(stands, 5)
                wrong_mode = 'creative' if mode == 'survival' else 'survival'
                mark = trace.mark()
                wrong = request('entity_interact_at', type=target_type, mode=wrong_mode, hit=[0., height, 0.])
                assert 'error' in wrong, wrong
                assert not frames(mark)
                native_before = command('data get entity ' + selector + ' equipment')
                mark = trace.mark()
                result = request('entity_interact_at', type=target_type, mode=mode, hit=[0., height, 0.], offhand=offhand)
                assert 'dispatch' in result, result
                sent = until(lambda: frames(mark), 5)
                assert len(sent) == 1, sent
                entity_id = result['target']['native_id']
                encoded_id = bytearray()
                while True:
                    byte = entity_id & 127
                    entity_id >>= 7
                    encoded_id.append(byte | (128 if entity_id else 0))
                    if not entity_id:
                        break
                expected = bytes(encoded_id) + b'\x02' + struct.pack('>fff', 0., height, 0.) + bytes([offhand, 0])
                assert sent[0]['body_hex'] == expected.hex(), sent
                def received_item():
                    # Modern living equipment stores offhand separately from Inventory.
                    value = {field: command('data get entity ClimbingProbe ' + field)
                             for field in ('Inventory', 'equipment')}
                    return value if wanted in json.dumps(value) else None
                player_after = until(received_item, 5)
                stand_after = command('data get entity ' + selector + ' equipment')
                assert retained in stand_after and wanted not in stand_after, stand_after
                assert wanted in json.dumps(player_after) and retained not in json.dumps(player_after), player_after
                command('kill ' + selector)
                until(lambda: not stands(), 8)
                mark = trace.mark()
                stale = request('entity_interact_at', saved=True, mode=mode, hit=[0., height, 0.], offhand=offhand)
                assert 'error' in stale, stale
                request('wait', ms=100)
                assert not frames(mark)
                report['checks'].append(dict(name=f'{mode}_{offhand}_{height}', result=result, original_packets=sent,
                                             native_before=native_before, native_stand_after=stand_after,
                                             native_player_after=player_after, wrong_mode=wrong, retired_target=stale,
                                             retired_target_packets=frames(mark)))
    trace.expect_disconnect(connection=1)
    request('disconnect')


def main():
    global BINARY
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('binary', 'jar', 'work'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--accept-eula', action='store_true', required=True)
    args = parser.parse_args()
    BINARY = args.binary.resolve()
    with tempfile.TemporaryDirectory(prefix='voxrig-entity-at-') as temporary:
        jars = Path(temporary)
        shutil.copyfile(args.jar.resolve(), jars / '1.21.11-server.jar')
        live.ROOT = jars / 'server'
        destination = args.work.resolve()
        destination.mkdir(parents=True, exist_ok=True)
        try:
            live.run('1.21.11', BINARY, jars, check=check)
        finally:
            for run in live.ROOT.glob('1.21.11-*'):
                retained = destination / run.name
                retained.mkdir(parents=True, exist_ok=True)
                for name in ('report.json', 'packets.jsonl', 'server.log', 'probe.log', 'probe.stderr.log'):
                    if (run / name).exists():
                        shutil.copyfile(run / name, retained / name)
    print(json.dumps({'retained': str(destination)}))


if __name__ == '__main__':
    main()
