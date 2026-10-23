#!/usr/bin/env python3
"""Check explicit plant replacement with raw receipts and independent vanilla RCON.

Runs only a disposable official 1.21.11 JAR copy; the shared harness owns and
stops all subprocesses. EULA acceptance is confined to that temporary directory.
"""
import argparse
import hashlib
import json
import shutil
import tempfile
from pathlib import Path
import run_climbing_control as harness
from run_common_native import until


def check(version, command, request, trace, report):
    assert version == '1.21.11'
    command('tp ClimbingProbe 0.5 65 0.5 -90 10')
    request('prepare', position=[.5, 65., .5])
    request('select_hotbar', slot=0)
    for plant in ('short_grass', 'fern', 'dead_bush'):
        for clicked in (True, False):
            command('fill 1 65 -1 3 68 1 minecraft:air')
            command('fill 1 65 -1 3 65 1 minecraft:dirt')
            destination = [2, 66, 0]
            support = [2, 66, 0] if clicked else [3, 66, 0]
            if not clicked:
                command('setblock 3 66 0 minecraft:stone')
            command(f'setblock {destination[0]} 66 0 minecraft:{plant}')
            # At the adjacent case the ray passes above the plant outline to the solid face.
            rotation = [-90., 10. if clicked else -8.]
            request('look', rotation=rotation)
            command('item replace entity ClimbingProbe weapon.mainhand with minecraft:dirt 1')
            until(lambda: (s := request('player')['inventory']['slots'][36]) and
                  s['value']['kind'] == 'item' and s['value']['item']['count'] == 1, 5)
            before = dict(name='minecraft:' + plant, properties={})
            # Vanilla synchronizes slots before its end-of-tick section changes.
            # Wait for the actual block predecessors as well as the material.
            def predecessors():
                check = request('placement_check', support=support, face='West')
                support_before = before if clicked else dict(name='minecraft:stone', properties={})
                return check if check['support_state'] == support_before and (
                    clicked or check['target_state'] == before) else None
            until(predecessors, 5)
            boundary = trace.mark()
            old = request('place_cube', clicked=support, face='West')
            assert 'error' in old, old
            wrong = request('place_cube_replacing', clicked=support, face='West',
                            target=destination, before=dict(name='minecraft:stone', properties={}))
            assert 'error' in wrong, wrong
            sent = request('place_cube_replacing', clicked=support, face='West', target=destination, before=before)
            assert 'record' in sent, sent
            sent = sent['record']
            assert sent['replaces_clicked'] == clicked and sent['before'] == before
            assert sent['send']['dispatched']

            def completed():
                r = request('replaceable_cube_placement_record')
                assert r['id'] == sent['id'] and r['stage'] != 'requires_inspection', r
                return r if r['stage'] == 'observed_placed' else None

            record = until(completed, 5)
            for field in ('target_receipt', 'material_receipt'):
                assert record[field]['source']['sequence'] > sent['send']['after_sequence']
            assert record['material_receipt']['value']['kind'] == 'empty'
            assert record['processing']['receive_sequence'] > sent['send']['after_sequence']
            block = command(f'execute if block {destination[0]} 66 0 minecraft:dirt')
            assert 'Test passed' in block, block
            inventory = command('data get entity ClimbingProbe SelectedItem')
            assert 'Found no elements' in inventory or inventory.endswith('{}'), inventory
            frames = trace.since(boundary)
            uses = [f for f in frames if f['direction'] == 'serverbound' and f['phase'] == 'play' and f['packet_id'] == 0x3f]
            assert len(uses) == 1, uses
            report['checks'].append(dict(plant=plant, replaces_clicked=clicked, sent=sent, complete=record,
                                         native_block=block, native_inventory=inventory, frames=frames))
    trace.expect_disconnect()
    request('disconnect')


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--accept-eula', action='store_true', required=True)
    p.add_argument('--server-jar', type=Path, required=True)
    p.add_argument('--binary', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    a = p.parse_args()
    a.output.mkdir(parents=True, exist_ok=True)
    original_port = harness.free_port
    def allowed_port():
        port = original_port()
        while port in (25572, 25581):
            port = original_port()
        return port
    harness.free_port = allowed_port
    with tempfile.TemporaryDirectory(prefix='voxrig-replaceable-') as temporary:
        root = Path(temporary)
        jars = root / 'jars'
        jars.mkdir()
        shutil.copyfile(a.server_jar, jars / '1.21.11-server.jar')
        harness.ROOT = root / 'runs'
        try:
            harness.run('1.21.11', a.binary.resolve(), jars, check=check)
        finally:
            if harness.ROOT.exists():
                for folder in harness.ROOT.iterdir():
                    for name in ('report.json', 'packets.jsonl', 'probe.log', 'probe.stderr.log', 'server.log'):
                        if (folder / name).exists():
                            shutil.copyfile(folder / name, a.output / name)
    (a.output / 'inputs.json').write_text(json.dumps(dict(binary_sha256=hashlib.sha256(a.binary.read_bytes()).hexdigest()), indent=2) + '\n')


if __name__ == '__main__':
    main()
