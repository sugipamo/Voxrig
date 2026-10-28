#!/usr/bin/env python3
"""Typed finite-motion interruption and control state log on SHA-1 verified
official servers (both versions).

Only original server packets and RCON commands: an RCON teleport while a
finite survival motion runs must retain a typed `correction` interruption with
the received pose beside the unchanged free-text problem; an RCON teleport to
the Nether while a PlayerControl owner runs must publish Started then
Stopped{world_changed} for that owner in `control_state_changes_after`.
"""
import argparse
import gzip
import hashlib
import json
from pathlib import Path
import sys

sys.dont_write_bytecode = True
import run_climbing_control as harness
from run_common_native import until


def check(version, command, request, trace, report):
    command('fill -8 64 -8 16 64 40 minecraft:stone')
    command('fill -8 65 -8 16 70 40 minecraft:air')

    # 1. Finite motion interrupted by a server correction (RCON teleport).
    command('tp ClimbingProbe .5 65 .5 0 0')
    assert 'error' not in request('prepare', position=[.5, 65., .5])
    controls = [dict(yaw=0., input=dict(forward=int(t < 10), strafe=0, jump=False))
                for t in range(35)]
    boundary = trace.mark()
    started = request('path_start', controls=controls)
    assert 'error' not in started, started
    until(lambda: (r if r['dispatched_ticks'] >= 2 else None)
          if (r := request('path_record')) else None, 8)
    command('tp ClimbingProbe 4.5 65 .5 0 0')
    record = until(lambda: (r if r['status'] != 'running' else None)
                   if (r := request('path_record')) else None, 8)
    interruption = record['interruption']
    assert record['run_id'] == started['run_id'], record
    assert record['status'] == 'requires_inspection', record
    assert record['problem'], record
    assert interruption and interruption['kind']['type'] == 'correction', record
    assert interruption['kind']['position'] == [4.5, 65.0, 0.5], record
    assert record['dispatched_ticks'] < len(controls), record
    report['checks'].append(dict(name='finite_motion_rcon_teleport_typed_correction',
                                 record=record, frames=trace.since(boundary)))
    request('wait', ms=300)

    # 2. World change (RCON teleport to the Nether) while a control owner runs.
    command('tp ClimbingProbe .5 65 .5 0 0')
    assert 'error' not in request('prepare', position=[.5, 65., .5])
    cursor = request('control_states')['sequence']
    owner = request('control_start')
    assert 'error' not in owner, owner
    keys = dict(forward=1, strafe=0, jump=False, sneak=False, sprint=False, yaw=0., pitch=0.)
    assert 'error' not in request('control_set', controls=keys)
    until(lambda: (r if r['dispatched_ticks'] >= 2 else None)
          if (r := request('control_record')) else None, 8)
    boundary = trace.mark()
    command('execute in minecraft:the_nether run tp ClimbingProbe 0.5 100 0.5')

    def stopped():
        log = request('control_states', after=cursor)
        mine = [c['transition'] for c in log['changes']
                if c['transition']['session_id'] == owner['session_id']]
        return (log, mine) if any(t['event']['state'] != 'started' for t in mine) else None

    log, mine = until(stopped, 10)
    events = [t['event'] for t in mine]
    assert [e['state'] for e in events] == ['started', 'stopped'], log
    assert events[1]['cause'] == 'world_changed', log
    assert events[1]['reason'].startswith('world changed'), log
    assert mine[1]['session']['world_generation'] == mine[0]['session']['world_generation'], log
    assert mine[1]['receive_sequence'] >= mine[0]['receive_sequence'], log
    final = request('control_record')
    assert final['session_id'] == owner['session_id'], final
    assert final['status']['status'] == 'stopped', final
    report['checks'].append(dict(name='control_world_change_stop_logged', owner=owner['session_id'],
                                 log=log, record=final, frames=trace.since(boundary)))
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
    sources = ['src/client/control.rs', 'src/client/control_states.rs',
               'src/client/survival/motion.rs',
               'src/versions/java_1_16_1/client/common_control.rs',
               'src/versions/java_1_16_1/client/common_motion.rs',
               'src/versions/java_1_21_11/client/operations/continuous.rs',
               'src/versions/java_1_21_11/client/operations/movement/control.rs',
               'examples/body_probe.rs', 'scripts/run_motion_control_stop_live.py', 'Cargo.lock']
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
            method='Original server packets only: a loopback RCON teleport during a finite '
            'survival motion, then an RCON teleport to the Nether during a PlayerControl owner. '
            'The typed interruption names the received pose, never a server-side cause.',
            checks=report['checks'], commands=report['commands'])
        with gzip.open(args.evidence, 'wt') as out:
            json.dump(public, out, ensure_ascii=False, indent=1)
    if not report['passed']:
        raise SystemExit(1)
