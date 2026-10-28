#!/usr/bin/env python3
"""Body UseItem inventory-resync scope on SHA-1 verified official servers (audit B12).

A container window's original WINDOW_ITEMS during the use must not report
Corrected{InventoryResync}; vanilla's own use-item resync of the player
inventory window (0) still does, also while a chest stays open.
Only original server packets and RCON commands; no synthetic replies/cache edits.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import sys

sys.dont_write_bytecode = True
import run_climbing_control as harness
from run_common_native import PacketTraceProxy, until

BASELINE = False  # --expect-overreport: pre-fix binary must reproduce the false correction


def check(version, command, request, trace, report):
    legacy = version == '1.16.1'
    window_items = 0x14 if legacy else 0x12
    use_packet = 0x2e if legacy else 0x40

    def complete(identity):
        assert not (isinstance(identity, dict) and 'error' in identity), identity
        result = until(lambda: s if (s := request('status', id=identity))['status'] != 'pending' else None, 12)
        request('wait', ms=80)
        return result

    def contents(frames):
        windows = []
        for f in frames:
            if f['direction'] == 'clientbound' and f['phase'] == 'play' and f['packet_id'] == window_items:
                raw = bytes.fromhex(f['body_hex'])
                windows.append(raw[0] if legacy else PacketTraceProxy.varint(raw)[0])
        return windows

    def hold(item, count):
        replace = 'replaceitem entity ClimbingProbe weapon.mainhand ' if legacy else \
            'item replace entity ClimbingProbe weapon.mainhand with '
        command(f'{replace}minecraft:{item} {count}')
        until(lambda: (v := (request('observe')['player']['inventory'][36] or {}).get('value')) and
              isinstance(v, dict) and v['Item']['name'] == 'minecraft:' + item and
              v['Item']['count'] == count, 8)

    def use(name, during=None):
        mark = trace.mark()
        identity = request('submit', kind='use')
        extra = request(during) if during else None
        result = complete(identity)
        frames = trace.since(mark)
        uses = [f for f in frames if f['direction'] == 'serverbound' and f['phase'] == 'play'
                and f['packet_id'] == use_packet]
        assert len(uses) == 1, uses
        record = dict(name=name, result=result, during=extra, windows=contents(frames), frames=frames)
        report['checks'].append(record)
        return record

    command('gamemode survival ClimbingProbe')
    command('fill -4 64 -4 4 64 4 minecraft:stone')
    command('fill -4 65 -4 4 69 4 minecraft:air')
    command('tp ClimbingProbe .5 65 .5 0 0')
    assert 'error' not in request('prepare', position=[.5, 65., .5])
    assert complete(request('submit', kind='select', slot=0))['status'] == 'accepted'
    hold('stick', 1)
    command('setblock 2 65 0 minecraft:chest[facing=west]')
    until(lambda: (b := request('observe', blocks=[[2, 65, 0]])['blocks'][0]['state']) and
          b['value']['name'] == 'minecraft:chest', 5)
    opened = complete(request('submit', kind='activate', pos=[2, 65, 0], face='West', cursor=[0., .5, .5]))
    assert opened['status'] == 'accepted', opened
    screen = until(lambda: s if (s := request('screen')).get('screen') else None, 6)
    # Change the open chest's content from the server, then let the server send
    # the whole container window while an ineffective item use (stick) is pending.
    replace = 'replaceitem block 2 65 0 container.0 ' if legacy else 'item replace block 2 65 0 container.0 with '
    command(replace + 'minecraft:diamond 3')
    request('wait', ms=200)
    unrelated = use('stick_use_with_container_window_items', during='inventory_resync')
    assert 'error' not in unrelated['during'], unrelated['during']
    assert unrelated['windows'] and 0 not in unrelated['windows'], unrelated['windows']
    unrelated['screen'] = screen
    if BASELINE:
        assert unrelated['result']['status'] == 'corrected', unrelated['result']
        assert unrelated['result']['correction'] == 'InventoryResync', unrelated['result']
    else:
        assert unrelated['result']['status'] == 'accepted', unrelated['result']

    # Regression: vanilla resyncs the player inventory window (0) after a use
    # that changed the stack (snowball count), even while the chest stays open.
    for name in ['snowball_use_with_chest_open', 'snowball_use_player_screen']:
        if name == 'snowball_use_player_screen':
            closed = request('close_screen')
            assert 'error' not in closed, closed
            request('wait', ms=200)
        hold('snowball', 4)
        real = use(name)
        assert 0 in real['windows'], real['windows']
        assert real['result']['status'] == 'corrected', real['result']
        assert real['result']['correction'] == 'InventoryResync', real['result']
    report['native_inventory'] = command('data get entity ClimbingProbe SelectedItem')
    trace.expect_disconnect()
    request('disconnect')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--accept-eula', action='store_true', required=True)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--jars', type=Path, required=True)
    parser.add_argument('--version', choices=['1.16.1', '1.21.11'], required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--expect-overreport', action='store_true',
                        help='assert the pre-fix false correction (baseline binary)')
    args = parser.parse_args()
    BASELINE = args.expect_overreport
    harness.ROOT = args.output.resolve()
    harness.run(args.version, args.binary.resolve(), args.jars.resolve(), check=check)
    report_path = sorted(harness.ROOT.glob('*/report.json'))[-1]
    report = json.loads(report_path.read_text())
    sources = ['src/body/headless.rs', 'src/versions/java_1_16_1/client/body.rs',
               'src/versions/java_1_21_11/client/operations/body.rs',
               'examples/body_probe.rs', 'scripts/run_body_useitem_resync_scope_live.py', 'Cargo.lock']
    report['baseline'] = BASELINE
    report['binary_sha256'] = hashlib.sha256(args.binary.read_bytes()).hexdigest()
    if BASELINE:  # the baseline binary is built from a different tree; record its commit instead
        report['source_commit'] = os.environ.get('VOXRIG_BASELINE_COMMIT')
    else:
        report['source_sha256'] = {p: hashlib.sha256((harness.REPO / p).read_bytes()).hexdigest() for p in sources}
    report_path.write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
    if not report['passed']:
        raise SystemExit(1)
