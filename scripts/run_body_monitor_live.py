#!/usr/bin/env python3
"""Body monitor completeness on a SHA-1 verified official Java 1.16.1 server.

Only original server packets and RCON commands; no synthetic replies/cache edits.
Timeout/read-error cases use deterministic fixtures instead of altering live state.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import sys
import time

sys.dont_write_bytecode = True
import run_climbing_control as harness
from run_common_native import until


def check(version, command, request, trace, report):
    assert version == '1.16.1'

    def complete(identity):
        outcome = until(lambda: s if (s := request('status', id=identity))['status'] != 'pending' else None, 5)
        assert outcome['status'] == 'accepted', outcome
        request('wait', ms=80)
        return request('action_observation', id=identity)

    command('tp ClimbingProbe .5 65 .5 0 0')
    request('prepare', position=[.5, 65., .5])
    sent = request('send_and_trust', kind='look', yaw=37., pitch=13.)
    assert sent['dispatch']['status'] == 'sent', sent
    first = complete(sent['id'])
    assert first['monitoring']['status'] == 'complete_through', first
    command('tp ClimbingProbe .5 65 .5 0 0')
    request('prepare_pose', position=[.5, 65., .5])
    events = request('events')  # refresh alone must publish the monitor transition
    corrected = request('action_observation', id=sent['id'])
    assert corrected['acceptance']['status'] == 'accepted', corrected
    assert corrected['outcome']['status'] == 'corrected', corrected
    assert corrected['monitoring']['status'] == 'correction_observed', corrected
    assert any(e['action'] == sent['id'] and e['status']['status'] == 'corrected' for e in events['events']), events
    report['checks'].append(dict(name='accepted_then_original_teleport_correction', first=first, observation=corrected, events=events))

    # An ordinary submit also owns monitoring, without changing its action_status.
    identity = request('submit', kind='select', slot=1)
    first = complete(identity)
    mark = trace.mark()
    # Keep each tick's edit below a full-section replacement threshold, and
    # separate ticks so vanilla cannot coalesce all intermediate updates.
    for index in range(80):
        name = 'gold_block' if index % 2 == 0 else 'diamond_block'
        command('fill 1 66 1 7 66 9 minecraft:' + name)  # 63 * 80 = 5040
        time.sleep(.06)
    until(lambda: (b := request('observe', blocks=[[7, 66, 9]])['blocks'][0]['state']) and b['value']['name'] == 'minecraft:diamond_block', 10)
    request('wait', ms=300)
    events = request('events')
    lost = request('action_observation', id=identity)
    assert lost['acceptance']['status'] == lost['outcome']['status'] == 'accepted', lost
    assert lost['monitoring']['status'] == 'gap', lost
    assert not events['gap'], events
    log = request('monitor_changes')
    assert any(c['action'] == identity and c['monitoring']['status'] == 'gap' for c in log['changes']), log
    assert request('action_observation', id=identity)['monitoring'] == lost['monitoring']
    report['checks'].append(dict(name='internal_evidence_gap_without_body_event_gap', first=first, observation=lost, log=log, frames=trace.since(mark)))

    identity = request('submit', kind='select', slot=2)
    first = complete(identity)
    # Advance a healthy monitor often enough to evict only the dedicated log.
    for index in range(12):
        name = 'gold_block' if index % 2 == 0 else 'diamond_block'
        command('setblock 0 66 2 minecraft:' + name)
        until(lambda: (b := request('observe', blocks=[[0, 66, 2]])['blocks'][0]['state']) and b['value']['name'] == 'minecraft:' + name, 5)
        assert request('action_observation', id=identity)['monitoring']['status'] == 'complete_through'
    log = request('monitor_changes', maximum=1)
    assert log['gap'] and len(log['changes']) == 1, log
    tail = request('monitor_changes', cursor=log['cursor'], maximum=4096)
    assert not tail['gap'] and len(tail['changes']) == 7, tail
    assert not request('monitor_changes', cursor=tail['cursor'])['changes']
    report['checks'].append(dict(name='dedicated_log_eviction_and_pagination', log=log, tail=tail))

    command('execute in minecraft:the_nether run tp ClimbingProbe .5 80 .5')
    lost = until(lambda: s if (s := request('action_observation', id=identity))['monitoring']['status'] == 'world_changed' else None, 10)
    assert lost['acceptance']['status'] == lost['outcome']['status'] == 'accepted', lost
    log = request('monitor_changes', cursor=tail['cursor'])
    assert any(c['action'] == identity and c['monitoring']['status'] == 'world_changed' for c in log['changes']), log
    assert request('action_observation', id=identity)['monitoring'] == lost['monitoring']
    report['checks'].append(dict(name='ordinary_submit_world_loss', first=first, observation=lost, log=log))
    trace.expect_disconnect()
    request('disconnect')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--accept-eula', action='store_true', required=True)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--jars', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    os.environ['VOXRIG_BODY_RETENTION'] = '8'
    harness.ROOT = args.output.resolve()
    harness.run('1.16.1', args.binary.resolve(), args.jars.resolve(), check=check)
    report_path = sorted(harness.ROOT.glob('*/report.json'))[-1]
    report = json.loads(report_path.read_text())
    sources = ['src/body/headless.rs', 'src/body/mod.rs', 'src/body/monitoring.rs',
               'src/body/headless/monitoring.rs', 'src/versions/java_1_16_1/client/body.rs',
               'examples/body_probe.rs', 'scripts/run_body_monitor_live.py', 'Cargo.lock']
    report['binary_sha256'] = hashlib.sha256(args.binary.read_bytes()).hexdigest()
    report['source_sha256'] = {p: hashlib.sha256((harness.REPO / p).read_bytes()).hexdigest() for p in sources}
    (harness.ROOT / 'report.json').write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
    if not report['passed']:
        raise SystemExit(1)
