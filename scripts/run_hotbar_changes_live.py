#!/usr/bin/env python3
"""Compare the selected-hotbar change log with original held-slot frames from SHA-1 checked vanilla servers.

Vanilla writes the selection from the server only on join, on dimension change /
cross-dimension teleport (PlayerList.sendAllPlayerInfo) and on pick-item. This
fixture uses join and RCON cross-dimension teleports as the server-initiated writes.
"""
import argparse
import gzip
import hashlib
import json
import re
import sys
import time
from pathlib import Path

sys.dont_write_bytecode = True
import run_climbing_control as harness
from run_common_native import until

IDS = {'1.16.1': (0x3f, 0x24), '1.21.11': (0x67, 0x34)}


def check(version, command, request, trace, report):
    clientbound, serverbound = IDS[version]

    def changes():
        log = request('hotbar_changes', after=0)
        assert 'error' not in log, log
        return log

    # Join: the server writes the saved selection (0 for a new player).
    until(lambda: changes()['changes'], 10)
    joined = changes()
    first = joined['changes'][0]['change']
    assert first['origin'] == 'Received' and first['before'] is None, joined
    assert first['after'] == 0, joined
    report['checks'].append(dict(name='join_received_selection', log=joined))

    def server_slot():
        text = command('data get entity ClimbingProbe SelectedItemSlot')
        match = re.search(r'data: (\d+)', text)
        if not match:
            raise RuntimeError('missing native selection: ' + text)
        return int(match[1])

    def received_after(cursor):
        log = request('hotbar_changes', after=cursor)
        hits = [c for c in log['changes'] if c['change']['origin'] == 'Received']
        return (log, hits) if hits else None

    selected = request('select_hotbar', slot=5)
    assert 'error' not in selected, selected
    submitted = changes()['changes'][-1]
    assert submitted['change']['origin'] == 'Submitted' and submitted['change']['after'] == 5, submitted
    assert submitted['change']['before'] == 0, submitted
    # Settle: the server has applied the submission before it writes again.
    until(lambda: server_slot() == 5, 10)

    # Server-initiated: cross-dimension teleport resends the server's selection.
    boundary = changes()['sequence']
    command('execute in minecraft:the_nether run tp ClimbingProbe 0.5 128 0.5')
    nether, hits = until(lambda: received_after(boundary), 15)
    assert all(h['change']['after'] == 5 for h in hits), nether
    assert all(h['change']['superseded_submission'] is None for h in hits), nether
    generation = submitted['change']['session']['world_generation']
    assert all(h['change']['session']['world_generation'] != generation for h in hits), nether
    report['checks'].append(dict(name='nether_teleport_received_selection', log=nether))

    boundary = nether['sequence']
    command('execute in minecraft:overworld run tp ClimbingProbe 0.5 65 0.5')
    back, hits = until(lambda: received_after(boundary), 15)
    assert all(h['change']['after'] == 5 for h in hits), back
    assert all(h['change']['superseded_submission'] is None for h in hits), back
    report['checks'].append(dict(name='overworld_teleport_received_selection', log=back))
    player = request('player')
    assert player['selected_hotbar']['value'] == 5, player
    # Wait for the new world to finish loading before the next submission.
    request('prepare', position=[0.5, 65.0, 0.5])

    # Race: submit and teleport without settling. Whichever order the server
    # applies them in, every write stays in the log; a received slot other than
    # the submission is reported as superseded_submission. Not asserted either way.
    boundary = changes()['sequence']
    assert 'error' not in request('select_hotbar', slot=2)
    command('execute in minecraft:the_nether run tp ClimbingProbe 0.5 128 0.5')
    raced, hits = until(lambda: received_after(boundary), 15)
    time.sleep(1)
    raced = request('hotbar_changes', after=boundary)
    local = request('player')['selected_hotbar']
    native = server_slot()
    superseded = [c for c in raced['changes'] if c['change']['superseded_submission'] is not None]
    for c in superseded:
        assert c['change']['origin'] == 'Received' and c['change']['after'] != c['change']['before'], c
    report['checks'].append(dict(name='race_submission_against_teleport', log=raced, local=local,
                                 server_slot=native, superseded=len(superseded),
                                 local_matches_server=local['value'] == native))

    final = changes()
    report['checks'].append(dict(name='final_log', log=final))
    trace.expect_disconnect()
    request('disconnect')
    frames = [f for f in trace.since(0) if f['phase'] == 'play']
    received_frames = [f for f in frames if f['direction'] == 'clientbound' and f['packet_id'] == clientbound]
    sent_frames = [f for f in frames if f['direction'] == 'serverbound' and f['packet_id'] == serverbound]
    report['checks'].append(dict(name='original_held_slot_frames',
                                 received=received_frames, sent=sent_frames))


def compare(report):
    """Every original held-slot frame matches exactly one log record, in order."""
    final = next(c['log'] for c in report['checks'] if c['name'] == 'final_log')
    records = [c['change'] for c in final['changes']]
    assert [c['sequence'] for c in final['changes']] == list(range(1, len(records) + 1)), final
    frames = next(c for c in report['checks'] if c['name'] == 'original_held_slot_frames')
    received = [r['after'] for r in records if r['origin'] == 'Received']
    submitted = [r['after'] for r in records if r['origin'] == 'Submitted']
    original_received = [bytes.fromhex(f['body_hex'])[0] for f in frames['received']]
    original_sent = [int.from_bytes(bytes.fromhex(f['body_hex'])[:2], 'big') for f in frames['sent']]
    assert received == original_received, (received, original_received)
    assert submitted == original_sent, (submitted, original_sent)
    assert all(a['after'] == b['before'] for a, b in zip(records, records[1:])), records
    report['checks'].append(dict(name='log_matches_original_frames', records=len(records),
                                 received=received, submitted=submitted))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--accept-eula', action='store_true', required=True)
    parser.add_argument('--version', choices=['1.16.1', '1.21.11'], required=True)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--jars', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    harness.ROOT = args.output.resolve()
    harness.run(args.version, args.binary.resolve(), args.jars.resolve(), check=check)
    report_path = sorted(harness.ROOT.glob(args.version + '-*/report.json'))[-1]
    report = json.loads(report_path.read_text())
    if report['passed']:
        compare(report)
    sources = ['src/client/hotbar_changes.rs', 'src/versions/java_1_16_1/client.rs',
               'src/versions/java_1_21_11/client.rs', 'src/versions/java_1_21_11/client/operations.rs',
               'examples/climbing_control_probe.rs', 'scripts/run_hotbar_changes_live.py', 'Cargo.lock']
    report['binary_sha256'] = hashlib.sha256(args.binary.read_bytes()).hexdigest()
    report['source_sha256'] = {p: hashlib.sha256((harness.REPO / p).read_bytes()).hexdigest() for p in sources}
    out = harness.ROOT / f'hotbar-changes-live-{args.version}.json.gz'
    out.write_bytes(gzip.compress((json.dumps(report, ensure_ascii=False, indent=2) + '\n').encode()))
    if not report['passed']:
        raise SystemExit(1)
