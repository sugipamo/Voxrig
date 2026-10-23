#!/usr/bin/env python3
"""Check crafting input SWAP with original packets and independent vanilla RCON.

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

    def empty(value):
        return value and value['value']['kind'] == 'empty'

    def exchange(mode, table, hotbar, phase, incoming, outgoing, result):
        baseline = slots(table)
        cursor = request('player')['inventory']['cursor']
        assert empty(cursor)
        boundary = trace.mark()
        sent = request('crafting_input_swap', slot=1, hotbar=hotbar, container=table)
        assert sent['send']['dispatched']
        destination = (37 if table else 36) + hotbar
        assert sent['hotbar_screen_slot'] == destination
        def completed():
            current = request('inventory_swap_record')
            assert current['id'] == sent['id'] and current['stage'] != 'requires_inspection', current
            return current if current['stage'] == 'observed_swapped' else None
        record = until(completed, 5)
        for name in ('source_receipt', 'hotbar_receipt'):
            assert record[name]['source']['sequence'] > sent['send']['after_sequence']
        actual = slots(table)
        assert (empty(actual[1]) if incoming is None else item(actual[1], *incoming)), actual[1]
        assert (empty(actual[destination]) if outgoing is None else item(actual[destination], *outgoing)), actual[destination]
        until(lambda: empty(slots(table)[0]) if result is None else item(slots(table)[0], *result), 5)
        if phase != 'empty-hotbar':
            assert slots(table)[0]['source']['sequence'] > sent['send']['after_sequence']
        assert request('player')['inventory']['cursor']['value'] == cursor['value']
        native = command(f'data get entity ClimbingProbe Inventory[{{Slot:{hotbar}b}}]')
        if outgoing:
            assert 'minecraft:' + outgoing[0] in native and 'count: ' + str(outgoing[1]) in native, native
            assert ('input_swap: 42' if outgoing[0]=='oak_planks' else 'swap_dirt: 43') in native, native
            if outgoing[0] == 'oak_planks':
                assert '"minecraft:max_stack_size": 16' in native, native
        else:
            assert 'No elements' in native or 'Found no elements' in native, native
        frames = trace.since(boundary)
        clicks = [f for f in frames if f['direction']=='serverbound' and f['phase']=='play' and f['packet_id']==0x11]
        assert len(clicks) == 1, clicks
        report['checks'].append(dict(mode=mode, table=table, hotbar=hotbar, phase=phase, baseline=baseline,
            cursor_before=cursor, cursor_after=request('player')['inventory']['cursor'],
            result_before=baseline[0], result_after=slots(table)[0], sent=sent, complete=record,
            native_inventory=native, frames=frames))

    for mode in ('survival', 'creative'):
        command('gamemode ' + mode + ' ClimbingProbe')
        until(lambda: request('player')['game_mode'] == mode, 5)
        for table in (False, True):
            command('clear ClimbingProbe')
            if not table:
                # OPEN invalidates player crafting inputs; close is not a receipt.
                # A data PICKUP/return requests actual player-zero full contents.
                command('item replace entity ClimbingProbe inventory.0 with minecraft:oak_planks[minecraft:custom_data={swap_baseline:44}] 1')
                until(lambda: item(slots(False)[9], 'oak_planks', 1), 5)
                request('inventory_click', slot=9)
                request('inventory_click', slot=9)
                until(lambda: all(empty(slots(False)[i]) for i in range(1,5)), 5)
                command('clear ClimbingProbe')
            if table:
                previous = (request('screen').get('screen') or {}).get('id')
                request('use_on_block', support=[0,65,2], face='North', cursor=[0.5,0.5,0.0])
                until(lambda: (request('screen').get('screen') or {}).get('id') not in (None,previous), 5)
            for hotbar in (0,8):
                command(f'item replace entity ClimbingProbe hotbar.{hotbar} with minecraft:oak_planks[minecraft:max_stack_size=16,minecraft:custom_data={{input_swap:42}}] 7')
                until(lambda: item(slots(table)[(37 if table else 36)+hotbar], 'oak_planks', 7), 5)
                exchange(mode,table,hotbar,'empty-input',('oak_planks',7),None,('oak_button',1))
                command(f'item replace entity ClimbingProbe hotbar.{hotbar} with minecraft:dirt[minecraft:custom_data={{swap_dirt:43}}] 3')
                until(lambda: item(slots(table)[(37 if table else 36)+hotbar], 'dirt', 3), 5)
                exchange(mode,table,hotbar,'occupied-input',('dirt',3),('oak_planks',7),None)
                command(f'item replace entity ClimbingProbe hotbar.{hotbar} with minecraft:air')
                until(lambda: empty(slots(table)[(37 if table else 36)+hotbar]), 5)
                exchange(mode,table,hotbar,'empty-hotbar',None,('dirt',3),None)
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
    with tempfile.TemporaryDirectory(prefix="voxrig-crafting-input-swap-") as temporary:
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
