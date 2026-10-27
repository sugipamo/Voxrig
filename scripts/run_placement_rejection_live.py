#!/usr/bin/env python3
"""Strict placement rejection on SHA-1 verified official Java 1.16.1 and 1.21.11 servers.

An armor stand occupying the target cell makes the original BlockItem refuse.
Only original server packets and RCON commands; no synthetic replies/cache edits.
RCON independently confirms the unchanged target and stack, then the released
owner places on the same connection once the obstruction is removed.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import sys

sys.dont_write_bytecode = True
import run_climbing_control as harness
from run_common_native import until

SUPPORT = [0, 64, -1]
TARGET = [0, 65, -1]
# Eye (0.5, 66.62, 0.5) to the support's top-face center: first outline is Up.
ROTATION = [180.0, 58.3]


def check(version, command, request, trace, report):
    def give():
        if version == '1.16.1':
            command('replaceitem entity ClimbingProbe hotbar.0 minecraft:dirt 5')
        else:
            command('item replace entity ClimbingProbe hotbar.0 with minecraft:dirt 5')

    def native_count():
        text = command('data get entity ClimbingProbe Inventory')
        match = re.search(r'(?:Count: |count: )(\d+)', text)
        return int(match[1]) if match and 'minecraft:dirt' in text else 0

    def target_is(name):
        return 'Test passed' in command('execute if block %d %d %d minecraft:%s' % (*TARGET, name))

    def resolved():
        record = request('placement_record')
        return record if record and record['stage'] != 'pending' else None

    command('kill @e[type=minecraft:armor_stand]')
    command('tp ClimbingProbe .5 65 .5 180 0')
    give()
    request('prepare', position=[.5, 65., .5])
    assert 'error' not in request('select', slot=0), 'select'
    command('summon minecraft:armor_stand .5 65 -.5 {NoGravity:1b,Invulnerable:1b}')
    request('wait', ms=500)
    request('strict_look', rotation=ROTATION)
    request('wait', ms=200)
    mark = trace.mark()
    sent = request('strict_place', pos=SUPPORT, face='Up')
    assert sent['send']['dispatched'] and sent['stage'] == 'pending', sent
    rejected = until(resolved, 10)
    frames = trace.since(mark)
    assert rejected['stage'] == 'observed_rejected', rejected
    assert rejected['rejection']['target_receipt']['value']['name'] == 'minecraft:air', rejected
    assert rejected['target_receipt'] is None and rejected['material_receipt'] is None, rejected
    if version == '1.16.1':
        assert rejected['processing'] is None and rejected['rejection']['support_receipt'], rejected
    else:
        assert rejected['processing']['sequence'] == rejected['send']['interaction_sequence'], rejected
        assert rejected['rejection']['boundary_receive_sequence'] == rejected['processing']['receive_sequence'], rejected
    assert target_is('air'), 'native target changed'
    assert native_count() == 5, 'native stack changed'
    report['checks'].append(dict(name='entity_obstructed_placement_rejected', sent=sent,
                                 record=rejected, frames=frames))

    # The rejected attempt released the owner: look and a fresh placement proceed.
    command('kill @e[type=minecraft:armor_stand]')
    request('wait', ms=500)
    look = request('strict_look', rotation=ROTATION)
    assert 'error' not in look, look
    request('wait', ms=200)
    placed_send = request('strict_place', pos=SUPPORT, face='Up')
    assert placed_send['id']['attempt'] != sent['id']['attempt'], placed_send
    placed = until(resolved, 10)
    assert placed['stage'] == 'observed_placed', placed
    assert target_is('dirt'), 'native target missing'
    assert native_count() == 4, 'native stack not consumed'
    report['checks'].append(dict(name='released_owner_places_after_obstruction_removed',
                                 look=look, sent=placed_send, record=placed))
    trace.expect_disconnect()
    request('disconnect')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--accept-eula', action='store_true', required=True)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--jars', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--version', choices=['1.16.1', '1.21.11'], action='append')
    args = parser.parse_args()
    harness.ROOT = args.output.resolve()
    sources = ['src/client/survival/placement.rs',
               'src/versions/java_1_16_1/client/common_placement.rs',
               'src/versions/java_1_21_11/client/operations/placement.rs',
               'examples/body_probe.rs', 'scripts/run_placement_rejection_live.py', 'Cargo.lock']
    summary = dict(binary_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest(),
                   source_sha256={p: hashlib.sha256((harness.REPO / p).read_bytes()).hexdigest() for p in sources},
                   versions={})
    for version in args.version or ['1.16.1', '1.21.11']:
        before = set(harness.ROOT.glob(version + '-*/report.json'))
        harness.run(version, args.binary.resolve(), args.jars.resolve(), check=check)
        report_path = sorted(set(harness.ROOT.glob(version + '-*/report.json')) - before)[-1]
        report = json.loads(report_path.read_text())
        summary['versions'][version] = dict(report=str(report_path), passed=report['passed'],
                                            server_sha1=report['server_sha1'])
        if not report['passed']:
            raise SystemExit(1)
    (harness.ROOT / 'summary.json').write_text(json.dumps(summary, ensure_ascii=False, indent=2) + '\n')
