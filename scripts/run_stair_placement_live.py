#!/usr/bin/env python3
"""Check checked dry oak stairs placement with raw receipts and independent vanilla RCON.

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
    command('tp ClimbingProbe 0.5 65 0.75 -90 10')
    request('prepare', position=[.5, 65., .75])
    request('select_hotbar', slot=0)
    def stair(facing, half, shape='straight'):
        return dict(name='minecraft:oak_stairs', properties=dict(facing=facing, half=half, shape=shape, waterlogged='false'))
    air = dict(name='minecraft:air', properties={})
    cases = []
    for facing, yaw, clicked, face, target in [
        ('east', -90., [3,66,0], 'West', [2,66,0]),
        ('south', 0., [0,66,3], 'North', [0,66,2]),
        ('west', 90., [-3,66,0], 'East', [-2,66,0]),
        ('north', 180., [0,66,-3], 'South', [0,66,-2]),
    ]:
        for half, pitch in [('bottom', 10.), ('top', -6.)]:
            cases.append((facing + '_' + half, clicked, face, [yaw,pitch], target, stair(facing,half), []))
    cases += [
        ('up_bottom_outer_left', [2,64,0], 'Up', [-90.,40.], [2,65,0], stair('east','bottom','outer_left'), [([3,65,0],stair('north','bottom'))]),
        ('up_bottom_outer_right', [2,64,0], 'Up', [-90.,40.], [2,65,0], stair('east','bottom','outer_right'), [([3,65,0],stair('south','bottom'))]),
        ('up_bottom_inner_left', [2,64,0], 'Up', [-90.,35.], [2,65,0], stair('east','bottom','inner_left'), [([1,65,0],stair('north','bottom'))]),
        ('up_bottom_inner_right', [2,64,0], 'Up', [-90.,35.], [2,65,0], stair('east','bottom','inner_right'), [([1,65,0],stair('south','bottom'))]),
        ('outer_blocked', [2,64,0], 'Up', [-90.,40.], [2,65,0], stair('east','bottom'), [([3,65,0],stair('north','bottom')),([2,65,1],stair('east','bottom'))]),
        ('down_top_outer_left', [2,68,0], 'Down', [-90.,-35.], [2,67,0], stair('east','top','outer_left'), [([3,67,0],stair('north','top'))]),
        ('neighbor_outer_change', [3,66,0], 'West', [-90.,-8.], [2,66,0], stair('east','top'), [([2,66,1],stair('north','top'))]),
        ('neighbor_inner_change', [3,66,0], 'West', [-90.,-8.], [2,66,0], stair('east','top'), [([2,66,1],stair('south','top'))]),
        ('two_neighbor_changes', [3,66,0], 'West', [-90.,-8.], [2,66,0], stair('east','top'), [([2,66,-1],stair('south','top')),([2,66,1],stair('south','top'))]),
        ('down_top', [2,68,0], 'Down', [-90.,-35.], [2,67,0], stair('east','top'), []),
    ]
    def block_arg(state):
        return state['name'] + '[' + ','.join(k+'='+v for k,v in state['properties'].items()) + ']'
    for name, clicked, face, rotation, target, expected, neighbors in cases:
        z = .25 if name == 'up_bottom_inner_right' else .75
        command(f'tp ClimbingProbe 0.5 65 {z} -90 10')
        request('prepare', position=[.5,65.,z])
        command('fill -4 65 -4 4 69 4 minecraft:air')
        command('setblock ' + ' '.join(map(str,clicked)) + ' minecraft:stone')
        for pos,state in neighbors:
            command('setblock ' + ' '.join(map(str,pos)) + ' ' + block_arg(state))
        request('look', rotation=rotation)
        command('item replace entity ClimbingProbe weapon.mainhand with minecraft:oak_stairs 1')
        until(lambda: (s := request('player')['inventory']['slots'][36]) and s['value']['kind'] == 'item'
              and s['value']['item']['name'] == 'minecraft:oak_stairs' and s['value']['item']['count'] == 1, 5)
        def baselines():
            c = request('placement_check', support=clicked, face=face)
            return c['target_state'] == air and c['support_state'] == dict(name='minecraft:stone', properties={})
        until(baselines, 5)
        # Wait for actual neighbor predecessors, independently of slot receipt.
        for pos,state in neighbors:
            until(lambda: request('placement_check', support=pos, face='Up')['support_state'] == state, 5)
        boundary = trace.mark()
        wrong = request('place_stair',clicked=clicked,face=face,target=target,before=dict(name='minecraft:stone',properties={}))
        assert 'error' in wrong,wrong
        result = request('place_stair',clicked=clicked,face=face,target=target,before=air)
        assert 'record' in result,(name,result)
        sent = result['record']
        assert sent['target'] == target and sent['expected'] == expected and sent['send']['dispatched'],(name,sent)
        def completed():
            r = request('stair_placement_record')
            assert r['id'] == sent['id'] and r['stage'] != 'requires_inspection',r
            return r if r['stage'] == 'observed_placed' else None
        record = until(completed,5)
        assert record['target_receipt']['value'] == expected,record
        for field in ('target_receipt','material_receipt'):
            assert record[field]['source']['sequence'] > sent['send']['after_sequence']
        assert record['material_receipt']['value']['kind'] == 'empty'
        assert record['processing']['receive_sequence'] > sent['send']['after_sequence']
        native_neighbors=[]
        for update in record['neighbor_updates']:
            assert update['receipt']['value'] == update['expected'],update
            assert update['receipt']['source']['sequence'] > sent['send']['after_sequence']
            result=command('execute if block ' + ' '.join(map(str,update['position'])) + ' ' + block_arg(update['expected']))
            assert 'Test passed' in result,result
            native_neighbors.append(result)
        if name in ('neighbor_outer_change', 'neighbor_inner_change'):
            assert len(record['neighbor_updates']) == 1,record
        if name == 'two_neighbor_changes':
            assert len(record['neighbor_updates']) == 2,record
        block = command('execute if block ' + ' '.join(map(str,target)) + ' ' + block_arg(expected))
        assert 'Test passed' in block,block
        inventory = command('data get entity ClimbingProbe SelectedItem')
        assert 'Found no elements' in inventory or inventory.endswith('{}'),inventory
        frames = trace.since(boundary)
        uses = [f for f in frames if f['direction'] == 'serverbound' and f['phase'] == 'play' and f['packet_id'] == 0x3f]
        assert len(uses) == 1,uses
        report['checks'].append(dict(name=name,sent=sent,complete=record,native_block=block,native_neighbors=native_neighbors,native_inventory=inventory,frames=frames))
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
    with tempfile.TemporaryDirectory(prefix='voxrig-stair-') as temporary:
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
