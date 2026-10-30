#!/usr/bin/env python3
"""Official 1.16.1 table craft -> close -> capture under original entity traffic."""
import argparse
import threading
import time
from pathlib import Path
import run_climbing_control as h
from run_common_native import until


def check(version, command, request, trace, report):
    assert version == '1.16.1'
    def item(value, name, count):
        return (value and value['value']['kind'] == 'item'
                and value['value']['item']['name'] == 'minecraft:' + name
                and value['value']['item']['count'] == count)

    def coherent(entry):
        cut = entry['capture']
        p, s, snapshot = cut['player'], cut['screen'], entry['snapshot']
        assert p['session'] == s['session'] == snapshot['session'], entry
        assert p['receive_sequence'] == s['receive_sequence'] == snapshot['receive_sequence'], entry
        assert p['inventory']['window_id'] == s['active_window'], entry
        assert p['inventory']['player_screen'] == s['player_screen'], entry
        assert p['inventory']['cursor'] == s['cursor'] == snapshot['cursor'], entry
        selection = entry['active_screen']
        assert selection['kind'] in ('open', 'closed'), entry
        active = selection.get('value')
        entry['open_screen'] = active
        assert active == snapshot['screen'], entry
        if active:
            # Constructor-verified mappings, including the actual original receipts.
            for mapping in active['layout']['player_slots']:
                assert p['inventory']['slots'][mapping['player_slot']] == active['slots'][mapping['screen_slot']], entry
        return entry

    command('tp ClimbingProbe 0.5 65 0.5 0 0')
    request('prepare', position=[.5, 65., .5])
    command('clear ClimbingProbe')
    command('setblock 1 65 0 minecraft:crafting_table')
    command('replaceitem entity ClimbingProbe inventory.0 minecraft:birch_log 1')
    until(lambda: item(request('player')['inventory']['slots'][9], 'birch_log', 1), 5)
    request('use_on_block', support=[1, 65, 0], face='Up', cursor=[.5, 1., .5])
    opened = until(lambda: (v if (v := coherent(request('capture_inventory_view')[0]))['open_screen'] else None), 5)
    request('inventory_click', slot=10, container=True)
    request('inventory_click', slot=1, container=True)
    until(lambda: item(request('crafting')['result'], 'birch_planks', 4), 5)
    sent = request('send_operational_crafting_result')
    crafted = until(lambda: (r if (r := request('operational_crafting_outcome'))['stage'] != 'pending' else None), 10)
    assert crafted['stage'] == 'crafted', crafted
    proven = coherent(request('capture_inventory_view')[0])
    assert proven['basis_matches'] is True, proven
    continued = request('continue_operational_crafting')
    assert continued['released'], continued
    request('inventory_click', slot=11, container=True)
    until(lambda: item(request('capture_inventory_view')[0]['capture']['player']['inventory']['slots'][10], 'birch_planks', 4), 5)
    native = command('data get entity ClimbingProbe Inventory[{Slot:10b}]')
    assert 'minecraft:birch_planks' in native and 'Count: 4b' in native, native
    close = request('close_screen')
    closed = coherent(request('capture_inventory_view')[0])
    assert closed['capture']['screen']['player_screen']['kind'] == 'submitted_close', closed
    assert closed['open_screen'] is None and closed['snapshot']['source']['kind'] == 'player', closed
    assert proven['open_screen'] == opened['open_screen'] or proven['open_screen']['id'] == opened['open_screen']['id']
    report['checks'].append(dict(name='table_craft_close_capture', opened=opened, sent=sent,
        crafted=crafted, proven=proven, continued=continued, native=native, close=close, closed=closed))

    # Each real vanilla entity is repeatedly moved on the isolated server; the
    # proxy observes original traffic and never fabricates a packet or inventory.
    for i in range(96):
        x, z = 4 + i % 8, 4 + i // 8
        command(f'summon minecraft:armor_stand {x}.5 65 {z}.5 {{Tags:["mo14","mo14_batch{i // 8}"],NoGravity:1b}}')
    mark = trace.mark()
    errors = []
    def churn():
        try:
            for i in range(24):
                step = '.125' if i % 2 == 0 else '-.125'
                # Keep each RCON response below its single-frame limit.
                for batch in range(12):
                    command(f'execute as @e[tag=mo14_batch{batch}] at @s run tp @s ~' + step + ' ~ ~')
                time.sleep(.03)
        except Exception as error:
            errors.append(repr(error))
    worker = threading.Thread(target=churn)
    worker.start()
    captures = [coherent(c) for c in request('capture_inventory_view', count=384)]
    worker.join()
    assert not errors, errors
    sequences = [c['capture']['player']['receive_sequence'] for c in captures]
    assert sequences[-1] > sequences[0], sequences
    assert all(c['open_screen'] is None for c in captures)
    frames = trace.since(mark)
    # 0x56 = original 1.16.1 Entity Teleport.
    teleports = [r for r in frames if r['phase'] == 'play' and r['direction'] == 'clientbound' and r['packet_id'] == 0x56]
    assert len(teleports) >= 96, len(teleports)
    report['checks'].append(dict(name='entity_burst_atomic_captures', entities=96,
        captures=captures, teleport_packets=len(teleports), sequence_range=[sequences[0], sequences[-1]]))
    trace.expect_disconnect()
    request('disconnect')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--accept-eula', action='store_true', required=True)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--jars', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    h.ROOT = args.output.resolve()
    h.run('1.16.1', args.binary.resolve(), args.jars.resolve(), check=check)
