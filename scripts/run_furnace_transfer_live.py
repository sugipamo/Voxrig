#!/usr/bin/env python3
"""Check furnace-family QUICK_MOVE with original packets and independent vanilla RCON.

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

    def slots():
        return request('screen')['screen']['slots']

    def item(value, name, count):
        return value and value['value']['kind'] == 'item' and value['value']['item']['name'] == 'minecraft:' + name and value['value']['item']['count'] == count

    def transfer(slot, expected_stage='observed_transferred'):
        boundary = trace.mark()
        sent = request('inventory_transfer', slot=slot, container=True)
        assert sent['send']['dispatched']
        def complete():
            record = request('inventory_transfer_record')
            assert record['id'] == sent['id']
            if expected_stage != 'requires_inspection':
                assert record['stage'] != 'requires_inspection', record
            return record if record['stage'] == expected_stage else None
        record = until(complete, 5)
        if expected_stage == 'observed_transferred':
            for change in record['changed_slots']:
                assert change['receipt']['source']['sequence'] > sent['send']['after_sequence']
                assert change['receipt']['value'] == change['prediction']['value']
        frames = trace.since(boundary)
        clicks = [f for f in frames if f['direction']=='serverbound' and f['phase']=='play' and f['packet_id']==0x11]
        assert len(clicks) == 1, clicks
        return dict(sent=sent, complete=record, frames=frames)

    for mode in ('survival', 'creative'):
        command('gamemode ' + mode + ' ClimbingProbe')
        until(lambda: request('player')['game_mode'] == mode, 5)
        for menu, material, output in [('furnace','raw_iron','iron_ingot'), ('blast_furnace','raw_iron','iron_ingot'), ('smoker','beef','cooked_beef')]:
            command('tick freeze')
            command('clear ClimbingProbe')
            command('setblock 0 65 2 minecraft:air')
            command('setblock 0 65 2 minecraft:' + menu)
            command('item replace entity ClimbingProbe inventory.0 with minecraft:coal 7')
            command('item replace entity ClimbingProbe inventory.1 with minecraft:' + material + ' 1')
            until(lambda: item(request('player')['inventory']['slots'][9], 'coal', 7) and item(request('player')['inventory']['slots'][10], material, 1), 5)
            previous = (request('screen').get('screen') or {}).get('id')
            request('use_on_block', support=[0,65,2], face='North', cursor=[0.5,0.5,0.0])
            until(lambda: request('screen').get('screen') and request('screen')['screen']['id'] != previous, 5)
            fuel = transfer(3)
            assert item(slots()[1], 'coal', 7) and slots()[3]['value']['kind']=='empty'
            materials = transfer(4)
            assert item(slots()[0], material, 1) and slots()[4]['value']['kind']=='empty'
            native_before = command('data get block 0 65 2 Items')
            assert 'minecraft:coal' in native_before and 'minecraft:' + material in native_before, native_before
            command('tick step 220')
            until(lambda: item(slots()[2], output, 1), 16)
            # Completed history remains immutable after autonomous smelting updates.
            historical = request('inventory_transfer_record')
            assert historical['stage']=='observed_transferred' and historical['id']==materials['sent']['id']
            result = transfer(2)
            assert item(slots()[38], output, 1) and slots()[2]['value']['kind']=='empty'
            native_after = command('data get entity ClimbingProbe Inventory')
            native_machine = command('data get block 0 65 2 Items')
            assert 'minecraft:' + output in native_after and 'minecraft:' + output not in native_machine
            report['checks'].append(dict(mode=mode, menu=menu, fuel=fuel, material=materials, output=result,
                native_before=native_before, native_inventory=native_after, native_machine=native_machine))
            if menu == 'furnace':
                command('item replace entity ClimbingProbe inventory.0 with minecraft:oak_log 7')
                until(lambda: item(slots()[3], 'oak_log', 7), 5)
                dual = transfer(3)
                assert item(slots()[0], 'oak_log', 7), dual
                # Recover input with forward order, before allowing any burn tick.
                recovered = transfer(0)
                assert item(slots()[3], 'oak_log', 7), recovered
                report['checks'][-1].update(dual_priority=dual, input_recovery=recovered)
            recovered_fuel = transfer(1)
            assert slots()[1]['value']['kind']=='empty'
            report['checks'][-1]['fuel_recovery'] = recovered_fuel
            request('close_screen')
    command('tick unfreeze')
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
    with tempfile.TemporaryDirectory(prefix="voxrig-furnace-transfer-") as temporary:
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
