#!/usr/bin/env python3
"""Real 1.16.1 inspection -> no-effect resync -> recovery -> new write.

The proxy only delays original frames. Vanilla owns all inventory and pickup
changes. RCON adds a stock change after the proven craft and before continuation.
"""
import argparse
import json
import threading
from pathlib import Path
import run_climbing_control as h
from run_common_native import PacketTraceProxy, until


class InflightTake(PacketTraceProxy):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self.armed = False
        self.held = []
        self.guard = threading.Lock()

    def forward_frame(self, record, data, writer):
        if self.armed and record['phase'] == 'play' and record['direction'] == 'serverbound' and record['packet_id'] == 0x09:
            body = bytes.fromhex(record['body_hex'])
            if int.from_bytes(body[1:3], 'big', signed=True) == 0:
                with self.guard:
                    self.held.append((record, data, writer))
                return
        super().forward_frame(record, data, writer)

    def release(self):
        self.armed = False
        with self.guard:
            for record, data, writer in self.held:
                super().forward_frame(record, data, writer)
            self.held.clear()


def check(version, command, request, trace, report):
    def item(value, name, count):
        return value and value['value']['kind'] == 'item' and value['value']['item']['name'] == 'minecraft:' + name and value['value']['item']['count'] == count

    command('tp ClimbingProbe 0.5 65 0.5 0 0')
    request('prepare', position=[.5, 65., .5])
    command('clear ClimbingProbe')
    command('replaceitem entity ClimbingProbe hotbar.0 minecraft:stone 1')
    command('replaceitem entity ClimbingProbe inventory.0 minecraft:oak_log 1')
    until(lambda: item(request('player')['inventory']['slots'][9], 'oak_log', 1), 5)
    request('inventory_click', slot=9)
    request('inventory_click', slot=1)
    until(lambda: item(request('crafting')['result'], 'oak_planks', 4), 5)
    mark = trace.mark()
    trace.armed = True
    sent = request('send_operational_crafting_result')
    until(lambda: len(trace.held) == 1, 5)
    command('summon minecraft:item 0.5 65 0.5 {Item:{id:"minecraft:oak_log",Count:1b},PickupDelay:0s}')
    until(lambda: [r for r in trace.since(mark) if r['direction'] == 'clientbound' and r['packet_id'] == 0x55], 10)
    trace.release()
    crafted = until(lambda: (r if (r := request('operational_crafting_outcome'))['stage'] != 'pending' else None), 10)
    assert crafted['stage'] == 'crafted' and len(crafted['external']) == 1, crafted
    # This changes actual server stock while the claimed owner still holds its gate.
    command('replaceitem entity ClimbingProbe inventory.3 minecraft:cobblestone 2')
    original = until(lambda: (r if (r := request('operational_crafting_outcome'))['stage'] == 'requires_inspection' else None), 10)
    assert original['external'] == crafted['external'], original
    strict = request('crafting_take_record')
    before = request('inventory_send_record')['record']
    resync = request('resync_inventory')
    request('capture_recovery_basis')
    # Pending rejection is covered deterministically in both packet fixtures;
    # a live server may finish its response before this command is serviced.
    complete = until(lambda: (r if (r := request('inventory_send_record')['record'])['status'] != 'pending' else None), 10)
    assert complete['status'] == 'received', complete
    basis = request('capture_recovery_basis')
    recovery = request('recover_inspected_crafting')
    assert recovery['kind'] == 'released', recovery
    receipt = recovery['recovery']
    assert receipt['original'] == original, receipt
    assert receipt['original_send'] == before, receipt
    assert receipt['original_take']['stage'] == strict['stage'], receipt
    assert receipt['basis_sequence'] == basis['sequence'], receipt
    assert item(receipt['basis']['cursor'], 'oak_planks', 4), receipt
    next_send = request('send_inventory_click', slot=10)
    next_complete = until(lambda: (r if (r := request('inventory_send_record')['record'])['status'] != 'pending' else None), 10)
    assert next_complete['status'] == 'received', next_complete
    player = request('player')
    assert item(player['inventory']['slots'][10], 'oak_planks', 4), player
    assert player['inventory']['cursor']['value']['kind'] == 'empty', player
    assert request('inspected_crafting_recovery') == receipt
    frames = trace.since(mark)
    slots = [int.from_bytes(bytes.fromhex(r['body_hex'])[1:3], 'big', signed=True) for r in frames if r['phase'] == 'play' and r['direction'] == 'serverbound' and r['packet_id'] == 0x09]
    assert slots == [0, -1, 10], slots
    report['checks'].append(dict(name='inspection_resync_recovery_next_write', sent=sent,
        crafted=crafted, original=original, strict=strict, original_send=before,
        resync=resync, complete=complete, recovery=recovery, next_send=next_send,
        next_complete=next_complete, player=player, native=command('data get entity ClimbingProbe Inventory'),
        clicked_slots=slots, frames=frames))
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
    h.run('1.16.1', args.binary.resolve(), args.jars.resolve(), check=check, trace_factory=InflightTake)
