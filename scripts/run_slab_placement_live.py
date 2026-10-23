#!/usr/bin/env python3
"""Check checked dry oak slab placement with raw receipts and independent vanilla RCON.

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
    def slab(kind):
        return dict(name='minecraft:oak_slab', properties=dict(type=kind, waterlogged='false'))
    cases = [
        ('side_bottom', [3,66,0], 'West', [-90.,10.], [2,66,0], None, 'bottom', False),
        ('side_top', [3,66,0], 'West', [-90.,-8.], [2,66,0], None, 'top', False),
        ('bottom_up_double', [2,65,0], 'Up', [-90.,30.], [2,65,0], 'bottom', 'double', True),
        ('top_down_double', [2,67,0], 'Down', [-90.,-30.], [2,67,0], 'top', 'double', True),
        ('adjacent_double', [3,66,0], 'West', [-90.,-8.], [2,66,0], 'bottom', 'double', False),
    ]
    for name, clicked, face, rotation, target, predecessor, kind, merges in cases:
        command('fill 1 65 -1 4 69 1 minecraft:air')
        if clicked != target or predecessor is None:
            command('setblock ' + ' '.join(map(str,clicked)) + ' minecraft:stone')
        if predecessor:
            command('setblock ' + ' '.join(map(str,target)) + f' minecraft:oak_slab[type={predecessor},waterlogged=false]')
        request('look', rotation=rotation)
        command('item replace entity ClimbingProbe weapon.mainhand with minecraft:oak_slab 1')
        until(lambda: (s := request('player')['inventory']['slots'][36]) and s['value']['kind'] == 'item'
              and s['value']['item']['name'] == 'minecraft:oak_slab' and s['value']['item']['count'] == 1, 5)
        before = slab(predecessor) if predecessor else dict(name='minecraft:air',properties={})
        def received_predecessor():
            # placement_check exposes both cells, including same-cell merging.
            check = request('placement_check',support=clicked,face=face)
            actual = check['support_state'] if clicked == target else check['target_state']
            return check if actual == before else None
        until(received_predecessor,5)
        boundary = trace.mark()
        wrong = request('place_slab',clicked=clicked,face=face,target=target,before=dict(name='minecraft:stone',properties={}))
        assert 'error' in wrong,wrong
        result = request('place_slab',clicked=clicked,face=face,target=target,before=before)
        assert 'record' in result,result
        sent = result['record']
        assert sent['target'] == target and sent['merges_clicked'] == merges,sent
        assert sent['expected'] == slab(kind) and sent['send']['dispatched'],sent
        def completed():
            r = request('slab_placement_record')
            assert r['id'] == sent['id'] and r['stage'] != 'requires_inspection',r
            return r if r['stage'] == 'observed_placed' else None
        record = until(completed,5)
        assert record['target_receipt']['value'] == slab(kind),record
        for field in ('target_receipt','material_receipt'):
            assert record[field]['source']['sequence'] > sent['send']['after_sequence']
        assert record['material_receipt']['value']['kind'] == 'empty'
        assert record['processing']['receive_sequence'] > sent['send']['after_sequence']
        block = command('execute if block ' + ' '.join(map(str,target)) + f' minecraft:oak_slab[type={kind},waterlogged=false]')
        assert 'Test passed' in block,block
        inventory = command('data get entity ClimbingProbe SelectedItem')
        assert 'Found no elements' in inventory or inventory.endswith('{}'),inventory
        frames = trace.since(boundary)
        uses = [f for f in frames if f['direction'] == 'serverbound' and f['phase'] == 'play' and f['packet_id'] == 0x3f]
        assert len(uses) == 1,uses
        report['checks'].append(dict(name=name,sent=sent,complete=record,native_block=block,native_inventory=inventory,frames=frames))
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
    with tempfile.TemporaryDirectory(prefix='voxrig-slab-') as temporary:
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
