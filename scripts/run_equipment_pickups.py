#!/usr/bin/env python3
"""Qualify exact equipment clicks and receipt records on both official servers."""
import argparse
import hashlib
from pathlib import Path
import re
import subprocess
import time

from run_climbing_control import REPO, run
from run_common_native import outer_snbt_compounds


def check(version, command, request, trace, report):
    report['synthetic_fixture_not_historical_reproduction'] = True
    command('tp ClimbingProbe 0.5 65 0.5 37 -12')
    request('prepare', position=[0.5, 65., 0.5])

    def wait_for(predicate):
        deadline = time.monotonic() + 5
        while True:
            state = request('player')
            if predicate(state):
                return state
            if time.monotonic() >= deadline:
                raise RuntimeError('fresh fixture inventory/mode receipt missing')
            time.sleep(0.02)

    def item(state, index):
        slot = state['inventory']['slots'][index]
        if slot is None or 'sequence' not in slot['source']:
            return None
        value = slot['value']
        return ('empty', 0) if value['kind'] == 'empty' else (value['item']['name'], value['item']['count'])

    def replace(slot, stack):
        command(f'replaceitem entity ClimbingProbe {slot} minecraft:{stack}' if version == '1.16.1'
                else f'item replace entity ClimbingProbe {slot} with minecraft:{stack}')

    def clicks_since(boundary):
        return [f for f in trace.since(boundary) if f['phase'] == 'play'
                and f['direction'] == 'serverbound' and f['packet_id'] == (0x09 if version == '1.16.1' else 0x11)]

    def native_inventory(expected):
        deadline = time.monotonic() + 2
        while True:
            text = command('data get entity ClimbingProbe Inventory')
            stacks = outer_snbt_compounds(text)
            matches = True
            equipment = {}
            for index, (name, count) in expected.items():
                raw = 108-index if 5 <= index <= 8 else -106 if index == 45 else index
                if version == '1.21.11' and index in (5, 6, 7, 8, 45):
                    field = {5:'head', 6:'chest', 7:'legs', 8:'feet', 45:'offhand'}[index]
                    # Modern vanilla saves equipment separately from Inventory.
                    equipment[index] = command(f'data get entity ClimbingProbe equipment.{field}')
                    actual = outer_snbt_compounds(equipment[index])
                else:
                    actual = [s for s in stacks if re.search(rf'Slot: {raw}b(?:,|\s|}})', s)]
                if count == 0:
                    matches &= not actual
                else:
                    matches &= len(actual) == 1 and f'id: "{name}"' in actual[0] and bool(
                        re.search(rf'(?:Count|count): {count}(?:b)?(?:,|\s|}})', actual[0]))
            if matches:
                return dict(inventory=text, equipment=equipment)
            if time.monotonic() >= deadline:
                raise RuntimeError('independent native equipment/inventory differs')
            time.sleep(0.05)

    def exchange(source, target):
        before = request('player')
        boundary = trace.mark()
        records = [request('inventory_click', slot=i) for i in [source, target, source]]
        after = request('player')
        if item(after, source) != item(before, target) or item(after, target) != item(before, source):
            raise RuntimeError('exact exchange used a different destination or lost stack count')
        if after['inventory']['cursor']['value']['kind'] != 'empty':
            raise RuntimeError('exact exchange did not empty the actual cursor')
        frames = clicks_since(boundary)
        if len(frames) != 3:
            raise RuntimeError('exact exchange did not send exactly three original clicks')
        for record in records:
            if record['stage'] != 'observed_clicked' or record['requires_inspection'] is not None:
                raise RuntimeError('click did not obtain both fresh original receipts')
            for field in ['source_receipt', 'cursor_receipt']:
                if record[field]['source']['sequence'] <= record['send']['after_sequence']:
                    raise RuntimeError('click result reused an old predecessor')
        return dict(before=before, records=records, after=after, original_frames=frames,
                    native_inventory=native_inventory({source:item(before, target), target:item(before, source)}))

    for mode in ['survival', 'creative']:
        command(f'gamemode {mode} ClimbingProbe')
        wait_for(lambda s: s['game_mode'].lower() == mode)
        command('clear ClimbingProbe')
        replace('inventory.4', 'shield 1')     # exact player slot 13
        replace('inventory.5', 'diamond_helmet 1')
        replace('weapon.offhand', 'stone 3')
        replace('armor.head', 'carved_pumpkin 1')
        wait_for(lambda s: all(item(s, i) == expected for i, expected in {
            13: ('minecraft:shield', 1), 14: ('minecraft:diamond_helmet', 1),
            45: ('minecraft:stone', 3), 5: ('minecraft:carved_pumpkin', 1), 8: ('empty', 0)}.items()))
        report['checks'].append(dict(name=mode + '_occupied_offhand_exact_exchange', **exchange(13, 45)))
        report['checks'].append(dict(name=mode + '_occupied_head_exact_exchange', **exchange(14, 5)))
        held = request('inventory_click', slot=14)
        boundary = trace.mark()
        before = request('inventory_record')
        rejected = request('inventory_click', slot=8, expect_rejected=True)
        if clicks_since(boundary) or request('inventory_record')['id'] != before['id']:
            raise RuntimeError('wrong armor item emitted a click or replaced the retained attempt')
        restored = request('inventory_click', slot=14)
        report['checks'].append(dict(name=mode + '_wrong_armor_item_rejected_before_write',
            held=held, rejection=rejected, restored=restored))

        before = request('player')['receive_sequence']
        curse = ('diamond_helmet{Enchantments:[{id:"minecraft:binding_curse",lvl:1s}]} 1'
                 if version == '1.16.1' else 'diamond_helmet[enchantments={"minecraft:binding_curse":1}] 1')
        replace('armor.head', curse)
        wait_for(lambda s: item(s, 5) == ('minecraft:diamond_helmet', 1)
                 and s['inventory']['slots'][5]['source']['sequence'] > before)
        boundary = trace.mark()
        if mode == 'survival':
            rejection = request('inventory_click', slot=5, expect_rejected=True)
            if clicks_since(boundary):
                raise RuntimeError('binding armor removal emitted a survival click')
            report['checks'].append(dict(name='survival_binding_armor_rejected_before_write', rejection=rejection))
        else:
            removed = request('inventory_click', slot=5)
            restored = request('inventory_click', slot=5)
            report['checks'].append(dict(name='creative_binding_armor_actual_removal_and_return',
                removed=removed, restored=restored, original_frames=clicks_since(boundary),
                native_inventory=native_inventory({5:('minecraft:diamond_helmet',1)})))
    report['authority_limits'] = ('Exact received slot/cursor predecessors, three independent PICKUP records '
        'for an explicitly chosen occupied exchange, no alternate equipment routing, both fresh receipts '
        'per click, original comparison replies and native inventory reads. Fixture changes are setup; '
        'receipts are not replaced by predictions. Three clicks are not an atomic swap, and cancellation '
        'or uncertainty requires inspection without replay. No Golemkit migration or historical replay claimed.')
    trace.expect_disconnect()
    request('disconnect')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--accept-eula', action='store_true', required=True)
    parser.add_argument('--version', choices=('1.16.1', '1.21.11'), action='append')
    parser.add_argument('--binary', type=Path, default=REPO / 'target/debug/examples/climbing_control_probe')
    parser.add_argument('--jars', type=Path)
    args = parser.parse_args()
    binary = args.binary.resolve()
    revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=REPO, text=True).strip()
    def recorded_check(*arguments):
        report = arguments[-1]
        report['source_revision'] = revision
        report['source_diff_sha256'] = hashlib.sha256(subprocess.check_output(['git', 'diff', 'HEAD'], cwd=REPO)).hexdigest()
        report['binary_sha256'] = hashlib.sha256(binary.read_bytes()).hexdigest()
        report['cargo_lock_sha256'] = hashlib.sha256((REPO / 'Cargo.lock').read_bytes()).hexdigest()
        check(*arguments)
    for version in args.version or ('1.16.1', '1.21.11'):
        run(version, binary, args.jars.resolve() if args.jars else None, check=recorded_check)


if __name__ == '__main__':
    main()
