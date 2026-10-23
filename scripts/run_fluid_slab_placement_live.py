#!/usr/bin/env python3
"""Verify liquid-aware oak slabs with raw packets and independent vanilla 1.21.11 RCON.

Only a disposable official JAR copy is run. The shared harness stops the probe,
proxy and server; EULA acceptance is confined to the temporary directory.
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
    # An elevated dry platform keeps the player's body above all basin fluids.
    command('setblock 0 67 0 minecraft:stone')
    command('tp ClimbingProbe 0.5 68 0.5 -90 53')
    request('prepare', position=[.5, 68., .5])
    request('select_hotbar', slot=0)

    def slab(kind, wet=False):
        return dict(name='minecraft:oak_slab', properties=dict(type=kind, waterlogged=str(wet).lower()))

    def water(level):
        return dict(name='minecraft:water', properties=dict(level=str(level)))

    def native(block):
        return block['name'] + ('[' + ','.join(k + '=' + v for k, v in block['properties'].items()) + ']' if block['properties'] else '')

    air = dict(name='minecraft:air', properties={})
    cases = [
        ('source_bottom', [3,66,0], 'West', [-90.,53.], [2,66,0], water(0), slab('bottom',True),False),
        ('source_top', [3,66,0], 'West', [-90.,50.], [2,66,0], water(0), slab('top',True),False),
        *[(f'flow_{level}', [3,66,0], 'West', [-90.,53.], [2,66,0], water(level), slab('bottom'),False) for level in (1,7,8)],
        ('dry_cell', [3,66,0], 'West', [-90.,53.], [2,66,0], air, slab('bottom'),False),
        ('wet_bottom_up_double', [2,66,0], 'Up', [-90.,57.34], [2,66,0], slab('bottom',True), slab('double'),True),
        ('wet_top_down_double', [2,70,0], 'Down', [-90.,-30.], [2,70,0], slab('top',True), slab('double'),True),
        ('wet_adjacent_double', [3,66,0], 'West', [-90.,50.], [2,66,0], slab('bottom',True), slab('double'),False),
        ('dry_bottom_up_double', [2,66,0], 'Up', [-90.,57.34], [2,66,0], slab('bottom'), slab('double'),True),
    ]
    for name, clicked, face, rotation, target, before, expected, merges in cases:
        command('fill 1 65 -2 4 72 9 minecraft:air')
        command('fill -1 68 -2 0 72 9 minecraft:air')
        if name.startswith('flow_'):
            # A one-cell-wide vanilla channel holds horizontal level 1 or 7.
            # A source directly above the destination holds falling level 8.
            command('fill 1 65 -1 3 65 8 minecraft:stone')
            command('fill 1 66 -1 1 66 8 minecraft:stone')
            command('fill 3 66 -1 3 66 8 minecraft:stone')
            command('setblock 2 66 -1 minecraft:stone')
            command('setblock 2 66 8 minecraft:stone')
            level = int(before['properties']['level'])
            source = [2,67,0] if level == 8 else [2,66,level]
            command('setblock ' + ' '.join(map(str,source)) + ' minecraft:water[level=0]')
            until(lambda: 'Test passed' in command('execute if block 2 66 0 ' + native(before)), 10)
        else:
            command('setblock 2 65 0 minecraft:stone')
            if clicked != target:
                command('setblock ' + ' '.join(map(str,clicked)) + ' minecraft:stone')
            if before != air:
                command('setblock ' + ' '.join(map(str,target)) + ' ' + native(before))
        request('look', rotation=rotation)
        command('item replace entity ClimbingProbe weapon.mainhand with minecraft:oak_slab 1')

        def ready():
            slot = request('player')['inventory']['slots'][36]
            if not (slot and slot['value']['kind'] == 'item' and slot['value']['item']['name'] == 'minecraft:oak_slab' and slot['value']['item']['count'] == 1):
                return None
            c = request('placement_check', support=clicked, face=face)
            return c if (c['support_state'] if clicked == target else c['target_state']) == before else None
        until(ready, 5)
        boundary = trace.mark()
        if before['name'] == 'minecraft:water' or before['properties'].get('waterlogged') == 'true':
            assert 'error' in request('place_slab', clicked=clicked, face=face, target=target, before=before)
        assert 'error' in request('place_slab_with_fluid', clicked=clicked, face=face, target=target, before=dict(name='minecraft:stone', properties={}))
        result = request('place_slab_with_fluid', clicked=clicked, face=face, target=target, before=before)
        assert 'record' in result, result
        sent = result['record']
        r = sent['placement']
        assert r['expected'] == expected and r['merges_clicked'] == merges and r['send']['dispatched'], sent
        assert sent['source']['receive_sequence'] == r['send']['after_sequence']
        assert sent['source']['connection_id'] == r['id']['session']['connection_id']
        if before['name'] == 'minecraft:water':
            level = int(before['properties']['level'])
            assert sent['before_fluid'] == dict(kind='water', source=level == 0, falling=level >= 8, amount=8 if level == 0 or level >= 8 else 8-level)
        assert sent['expected_fluid'] == (dict(kind='water', source=True, falling=False, amount=8) if expected['properties']['waterlogged'] == 'true' else dict(kind='empty'))

        def completed():
            capture = request('fluid_slab_placement_record')
            record = capture['placement']
            assert record['id'] == r['id'] and record['stage'] != 'requires_inspection', capture
            return capture if record['stage'] == 'observed_placed' else None
        complete = until(completed, 5)
        record = complete['placement']
        assert record['target_receipt']['value'] == expected
        assert record['material_receipt']['value']['kind'] == 'empty'
        for field in ('target_receipt', 'material_receipt'):
            assert record[field]['source']['sequence'] > r['send']['after_sequence']
        assert record['processing']['receive_sequence'] > r['send']['after_sequence']
        block = command('execute if block ' + ' '.join(map(str,target)) + ' ' + native(expected))
        assert 'Test passed' in block, block
        inventory = command('data get entity ClimbingProbe SelectedItem')
        assert 'Found no elements' in inventory or inventory.endswith('{}'), inventory
        frames = trace.since(boundary)
        uses = [f for f in frames if f['direction'] == 'serverbound' and f['phase'] == 'play' and f['packet_id'] == 0x3f]
        assert len(uses) == 1, uses
        evidence = dict(name=name, sent=sent, complete=complete, native_block=block, native_inventory=inventory, frames=frames)
        if name.startswith('flow_'):
            supply = command('execute if block ' + ' '.join(map(str,source)) + ' minecraft:water[level=0]')
            assert 'Test passed' in supply, supply
            evidence['fluid_observation'] = dict(source_position=source, native_source_preserved=supply)
        if name == 'source_bottom':
            # The saved outcome must remain unchanged across native neighboring
            # fluid updates. Those updates are not additional success receipts.
            mark = trace.mark()
            neighbor = until(lambda: (s if 'Test passed' in (s := command('execute if block 2 65 1 minecraft:water')) else None), 5)
            assert request('fluid_slab_placement_record') == complete
            retained = command('execute if block ' + ' '.join(map(str,target)) + ' ' + native(expected))
            assert 'Test passed' in retained, retained
            evidence['fluid_observation'] = dict(native_neighbor_water=neighbor, native_retained_waterlogged_slab=retained, frames=trace.since(mark))
        report['checks'].append(evidence)
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
    with tempfile.TemporaryDirectory(prefix='voxrig-fluid-slab-') as temporary:
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
