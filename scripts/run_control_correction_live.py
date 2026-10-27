#!/usr/bin/env python3
"""PlayerControl correction log on SHA-1 verified official servers (both versions).

Only original server packets and RCON commands: two back-to-back RCON teleports
while a strict control owner runs, then server-side movement rejection provoked
by a high movement-speed attribute ("moved too quickly"). The log records a
superseded submission and its delta, never a teleport/rejection cause.
"""
import argparse
import gzip
import hashlib
import json
import math
from pathlib import Path
import re
import sys

sys.dont_write_bytecode = True
import run_climbing_control as harness
from run_common_native import until


def check(version, command, request, trace, report):
    modern = version == '1.21.11'
    speed = 'minecraft:movement_speed' if modern else 'minecraft:generic.movement_speed'
    command('fill -8 64 -8 16 64 40 minecraft:stone')
    command('fill -8 65 -8 16 70 40 minecraft:air')
    keys = dict(forward=1, strafe=0, jump=False, sneak=False, sprint=False, yaw=0., pitch=0.)

    def position():
        raw = command('data get entity ClimbingProbe Pos')
        return [float(v.strip().rstrip('df')) for v in re.search(r'\[([^]]+)\]', raw)[1].split(',')]

    def start(kind='best_effort_start'):
        command('tp ClimbingProbe .5 65 .5 0 0')
        assert 'error' not in request('prepare', position=[.5, 65., .5])
        record = request(kind)
        assert 'error' not in record, record
        assert 'error' not in request('control_set', controls=keys)
        first = record['dispatched_ticks']
        record = until(lambda: (r if r['dispatched_ticks'] >= first + 3 else None)
                       if (r := request('control_record')) else None, 8)
        assert record['status']['status'] == 'running', record
        return record

    def owned(cursor, owner, count, accept=lambda c: True):
        def ready():
            log = request('control_corrections', after=cursor)
            mine = [c for c in log['corrections']
                    if c['correction']['session_id'] == owner and accept(c['correction'])]
            return (log, mine) if len(mine) >= count else None
        return until(ready, 10)

    # 0. Strict owners keep their existing policy: a correction that discards
    # the contact origin stops them, and that stopping receipt is still logged.
    record = start('control_start')
    owner = record['session_id']
    cursor = request('control_corrections')['sequence']
    command('tp ClimbingProbe 4.5 65 .5 0 0')
    log, mine = owned(cursor, owner, 1)
    stopped = until(lambda: (r if r['status']['status'] == 'stopped' else None)
                    if (r := request('control_record')) else None, 8)
    assert stopped['session_id'] == owner, stopped
    assert mine[0]['correction']['received_position'][0] == 4.5, mine
    report['checks'].append(dict(name='strict_owner_stopping_correction_logged', owner=owner,
                                 log=log, record=stopped))
    request('control_stop')
    request('wait', ms=200)

    # 1. Two RCON teleports sent back to back while a best-effort owner walks.
    record = start()
    owner = record['session_id']
    cursor = request('control_corrections')['sequence']
    boundary = trace.mark()
    command('tp ClimbingProbe 4.5 65 .5 0 0')
    command('tp ClimbingProbe 8.5 65 .5 0 0')
    log, mine = owned(cursor, owner, 2)
    first, second = mine[0]['correction'], mine[1]['correction']
    assert first['receive_sequence'] < second['receive_sequence'], mine
    assert [first['received_position'][0], second['received_position'][0]] == [4.5, 8.5], mine
    assert first['submitted'] is not None and first['session']['version'] == (
        'Java1_21_11' if modern else 'Java1_16_1'), first
    assert math.dist(second['model_position'], first['received_position']) < 1., mine
    after = request('control_record')
    assert after['session_id'] == owner and after['status']['status'] == 'running', after
    later = until(lambda: (r if r['dispatched_ticks'] > after['dispatched_ticks'] else None)
                  if (r := request('control_record')) else None, 8)
    assert later['status']['status'] == 'running', later
    report['checks'].append(dict(
        name='back_to_back_rcon_teleports_logged_while_running', owner=owner, log=log,
        record_before=record, record_after=later,
        applied_corrections=later['corrections'] - record['corrections'],
        logged_corrections=len(mine), frames=trace.since(boundary)))
    assert 'error' not in request('control_stop')
    request('wait', ms=200)

    # 2. Server-side movement rejection: the client model honours the received
    # speed attribute and submits >10 blocks per tick; the server snaps it back.
    command(f'attribute ClimbingProbe {speed} base set 10')
    request('wait', ms=300)
    log_path = sorted(harness.ROOT.glob(version + '-*/server.log'))[-1]
    warnings_before = log_path.read_text(errors='replace').count('moved too quickly')
    record = start()
    owner = record['session_id']
    cursor = request('control_corrections')['sequence']
    boundary = trace.mark()

    def rejected(c):
        s = c['submitted']
        return s is not None and math.dist(s['position'], c['received_position']) > 5.

    log, mine = owned(cursor, owner, 3, rejected)
    after = request('control_record')
    assert after['session_id'] == owner and after['status']['status'] == 'running', after
    assert 'error' not in request('control_stop')
    native = position()
    command(f'attribute ClimbingProbe {speed} base set 0.1')
    request('wait', ms=300)
    warnings = log_path.read_text(errors='replace').count('moved too quickly') - warnings_before
    # Each vanilla rejection is answered by one position packet: the log must
    # contain at least as many far superseded submissions as server warnings
    # issued before the stop (later ones arrive after the owner stopped).
    final = request('control_corrections', after=cursor)
    far = [c for c in final['corrections'] if c['correction']['session_id'] == owner
           and rejected(c['correction'])]
    assert warnings > 0 and len(far) >= 3, (warnings, far)
    report['checks'].append(dict(
        name='server_movement_rejection_logged_while_running', owner=owner, log=log,
        rejections=[c['correction'] for c in mine], record_after=after, native_position=native,
        server_moved_too_quickly_warnings=warnings, final_log=final, frames=trace.since(boundary)))
    trace.expect_disconnect()
    request('disconnect')


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--accept-eula', action='store_true', required=True)
    p.add_argument('--binary', type=Path, required=True)
    p.add_argument('--jars', type=Path, required=True)
    p.add_argument('--version', choices=['1.16.1', '1.21.11'], required=True)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--evidence', type=Path, help='write a gzip public evidence summary')
    args = p.parse_args()
    harness.ROOT = args.output.resolve()
    harness.run(args.version, args.binary.resolve(), args.jars.resolve(), check=check)
    report_path = sorted(harness.ROOT.glob(args.version + '-*/report.json'))[-1]
    report = json.loads(report_path.read_text())
    sources = ['src/client/control.rs', 'src/client/control_changes.rs',
               'src/versions/java_1_16_1/client.rs',
               'src/versions/java_1_16_1/client/common_control.rs',
               'src/versions/java_1_21_11/client.rs',
               'src/versions/java_1_21_11/client/operations/continuous.rs',
               'examples/body_probe.rs', 'scripts/run_control_correction_live.py', 'Cargo.lock']
    report['binary_sha256'] = hashlib.sha256(args.binary.read_bytes()).hexdigest()
    report['source_sha256'] = {s: hashlib.sha256((harness.REPO / s).read_bytes()).hexdigest()
                               for s in sources}
    report_path.write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
    if args.evidence:
        packets = report_path.parent / 'packets.jsonl'
        public = dict(
            version=report['version'], passed=report['passed'], server_sha1=report['server_sha1'],
            binary_sha256=report['binary_sha256'], source_sha256=report['source_sha256'],
            raw_report_sha256=hashlib.sha256(report_path.read_bytes()).hexdigest(),
            raw_packets_sha256=hashlib.sha256(packets.read_bytes()).hexdigest()
            if packets.exists() else None,
            correction_method='Original server packets only: two loopback RCON teleports, then '
            'vanilla "moved too quickly" rejection provoked by an RCON movement-speed attribute. '
            'The log reports superseded submissions and deltas, never a cause.',
            checks=report['checks'], commands=report['commands'])
        with gzip.open(args.evidence, 'wt') as out:
            json.dump(public, out, ensure_ascii=False, indent=1)
    if not report['passed']:
        raise SystemExit(1)
