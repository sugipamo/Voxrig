#!/usr/bin/env python3
"""Check crafting input QUICK_MOVE with original packets and independent vanilla RCON.

Only a temporary copy of the supplied 1.21.11 JAR is run. All processes and
credentials are owned and cleaned up by run_climbing_control.run.
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
    command('tp ClimbingProbe 0.5 65 0.5 0 0')
    request('prepare', position=[0.5, 65.0, 0.5])
    request('select_hotbar', slot=0)
    command('setblock 0 65 2 minecraft:crafting_table')

    def slots(table):
        return request('screen')['screen']['slots'] if table else request('player')['inventory']['slots']

    def item(value, name, count):
        return value and value['value']['kind'] == 'item' and value['value']['item']['name'] == 'minecraft:' + name and value['value']['item']['count'] == count

    for mode in ('survival', 'creative'):
        command('gamemode ' + mode + ' ClimbingProbe')
        until(lambda: request('player')['game_mode'] == mode, 5)
        for table in (False, True):
            for partial in (False, True):
                command('clear ClimbingProbe')
                command('item replace entity ClimbingProbe inventory.0 with minecraft:oak_planks[minecraft:custom_data={input_transfer:42}] 7')
                command('item replace entity ClimbingProbe inventory.2 with minecraft:diamond_helmet[minecraft:damage=7,minecraft:custom_data={input_transfer_cursor:43}] 1')
                until(lambda: item(request('player')['inventory']['slots'][9], 'oak_planks', 7), 5)
                if table:
                    previous = (request('screen').get('screen') or {}).get('id')
                    request('use_on_block', support=[0,65,2], face='North', cursor=[0.5,0.5,0.0])
                    def new_opening():
                        screen = request('screen').get('screen')
                        return screen if screen and screen['id'] != previous else None
                    until(new_opening, 5)
                start = 10 if table else 9
                # Ordinary PICKUP supplies actual grid and cursor receipts.
                request('inventory_click', slot=start, container=table)
                request('inventory_click', slot=1, container=table)
                until(lambda: item(slots(table)[1], 'oak_planks', 7), 5)
                # A data-bearing held cursor must survive both full and partial transfers.
                request('inventory_click', slot=start + 2, container=table)
                if partial:
                    for i in range(36):
                        destination = 'inventory.' + str(i) if i < 27 else 'hotbar.' + str(i - 27)
                        command('item replace entity ClimbingProbe ' + destination + ' with minecraft:dirt 64')
                    for index, count in ((0,63),(1,62)):
                        command(f'item replace entity ClimbingProbe inventory.{index} with minecraft:oak_planks[minecraft:custom_data={{input_transfer:42}}] {count}')
                    until(lambda: item(slots(table)[start], 'oak_planks', 63) and item(slots(table)[start + 1], 'oak_planks', 62) and item(slots(table)[start + 35], 'dirt', 64), 5)
                baseline = slots(table)
                cursor_before = request('player')['inventory']['cursor']
                boundary = trace.mark()
                sent = request('inventory_transfer', slot=1, container=table)
                assert sent['send']['dispatched'] and sent['send']['request_full_resync']

                def complete():
                    record = request('inventory_transfer_record')
                    assert record['id'] == sent['id']
                    assert record['stage'] != 'requires_inspection', record
                    return record if record['stage'] == 'observed_transferred' else None

                record = until(complete, 5)
                assert len(record['changed_slots']) == (3 if partial else 2), record
                for change in record['changed_slots']:
                    receipt = change['receipt']
                    assert receipt['source']['sequence'] > sent['send']['after_sequence']
                    assert receipt['value'] == change['prediction']['value']
                assert record['cursor_inspected']['value'] == cursor_before['value']
                actual = slots(table)
                if partial:
                    assert item(actual[1], 'oak_planks', 4) and item(actual[start], 'oak_planks', 64) and item(actual[start+1], 'oak_planks', 64)
                else:
                    assert actual[1]['value']['kind'] == 'empty' and item(actual[start], 'oak_planks', 7)
                native = command('data get entity ClimbingProbe Inventory')
                assert 'input_transfer: 42' in native and ('count: 64' if partial else 'count: 7') in native, native
                frames = trace.since(boundary)
                clicks = [f for f in frames if f['direction']=='serverbound' and f['phase']=='play' and f['packet_id']==0x11]
                assert len(clicks) == 1, clicks
                report['checks'].append(dict(mode=mode, table=table, partial=partial, baseline=baseline,
                    cursor_before=cursor_before, result_before=baseline[0], result_after=actual[0], sent=sent, complete=record, native_inventory=native, frames=frames))
                # Free one destination via RCON, then return the actually held helmet.
                command('item replace entity ClimbingProbe inventory.2 with minecraft:air')
                until(lambda: slots(table)[start+2]['value']['kind']=='empty', 5)
                request('inventory_click', slot=start+2, container=table)
                helmet = command('data get entity ClimbingProbe Inventory[{Slot:11b}]')
                assert 'input_transfer_cursor: 43' in helmet and '"minecraft:damage": 7' in helmet, helmet
                report['checks'][-1]['native_returned_cursor'] = helmet
                if partial:
                    # Explicitly recover the retained input after clearing one slot.
                    command('item replace entity ClimbingProbe inventory.0 with minecraft:air')
                    until(lambda: slots(table)[start]['value']['kind']=='empty', 5)
                    request('inventory_click', slot=1, container=table)
                    request('inventory_click', slot=start, container=table)
                if table:
                    request('close_screen')
    trace.expect_disconnect()
    request('disconnect')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--accept-eula", action="store_true", required=True)
    parser.add_argument("--server-jar", type=Path, required=True)
    parser.add_argument("--binary", type=Path, default=harness.REPO / "target/debug/examples/climbing_control_probe")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    original_free_port = harness.free_port
    def disposable_port():
        port = original_free_port()
        while port in (25572, 25581):
            port = original_free_port()
        return port
    harness.free_port = disposable_port
    # The caller's server folder remains read-only, including unpacked libraries.
    with tempfile.TemporaryDirectory(prefix="voxrig-crafting-input-") as temporary:
        root = Path(temporary)
        jars = root / "jars"
        jars.mkdir()
        shutil.copyfile(args.server_jar, jars / "1.21.11-server.jar")
        harness.ROOT = root / "runs"
        try:
            harness.run("1.21.11", args.binary.resolve(), jars, check=check)
        finally:
            if harness.ROOT.exists():
                for folder in harness.ROOT.iterdir():
                    # Preserve reviewable evidence, never server credentials or worlds.
                    for name in ("report.json", "packets.jsonl", "probe.log", "probe.stderr.log", "server.log"):
                        source = folder / name
                        if source.exists():
                            shutil.copyfile(source, args.output / name)
    sources = {}
    for base in ("src", "data"):
        for path in sorted((harness.REPO / base).rglob("*")):
            if path.is_file():
                sources[str(path.relative_to(harness.REPO))] = hashlib.sha256(path.read_bytes()).hexdigest()
    sources["examples/climbing_control_probe.rs"] = hashlib.sha256(
        (harness.REPO / "examples/climbing_control_probe.rs").read_bytes()).hexdigest()
    (args.output / "inputs.json").write_text(json.dumps(dict(
        sources=sources, binary_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest()), indent=2) + "\n")


if __name__ == "__main__":
    main()
