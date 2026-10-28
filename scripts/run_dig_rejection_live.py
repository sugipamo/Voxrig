#!/usr/bin/env python3
"""Server-rejected digging on SHA-1 verified official Java 1.16.1 and 1.21.11 servers.

A world border that excludes the target cell makes `Level.mayInteract` refuse
START. Only original server packets and RCON commands; no synthetic replies or
cache edits. RCON independently confirms the unchanged target, then the same
connection digs it once the border is lifted (success regression). A refused
START on the cell the server last destroyed is then shown to still break on STOP,
so the refused START's echo alone is never a rejection. On 1.21.11 the common
staged mining attempt on a fresh cell is refused and ends `observed_rejected`.
"""
import argparse
import hashlib
import json
from pathlib import Path
import sys

sys.dont_write_bytecode = True
import run_climbing_control as harness
from run_common_native import until

TARGET = [0, 65, 2]
# Common mining uses a fresh cell; the server keeps its last destroy position.
MINED = [1, 65, 2]
# Eye (0.5, 66.62, 0.5) to the north-face center of MINED: first outline is North.
ROTATION = [-33.69, 31.85]


def check(version, command, request, trace, report):
    legacy = version == '1.16.1'
    dig = 0x1b if legacy else 0x28
    block_update, ack = (0x0b, 0x07) if legacy else (0x08, 0x04)

    def target_name(at):
        state = request('observe', blocks=[at])['blocks'][0]['state']
        return state['value']['name'] if state else None

    def setblock(name, at=TARGET):
        command('setblock %d %d %d minecraft:%s' % (*at, name))
        until(lambda: target_name(at) == 'minecraft:' + name, 5)

    def target_is(name, at=TARGET):
        return 'Test passed' in command('execute if block %d %d %d minecraft:%s' % (*at, name))

    def frames(mark):
        # Target dig commands and their original replies, in wire order.
        out = []
        for f in trace.since(mark):
            if f['phase'] != 'play':
                continue
            key = (f['direction'], f['packet_id'])
            if key in {('serverbound', dig), ('clientbound', block_update), ('clientbound', ack)}:
                out.append(dict(ordinal=f['ordinal'], direction=f['direction'],
                                packet_id=hex(f['packet_id']), body_hex=f['body_hex']))
        return out

    def border(enclosed):
        if enclosed:
            # Bounds [-1, 2) on x and z: the player at z=0.5 is inside, z=2 is not.
            command('worldborder center 0.5 0.5')
            command('worldborder set 3')
        else:
            command('worldborder center 0 0')
            command('worldborder set 59999968')
        request('wait', ms=300)

    command('fill -3 64 -3 3 64 4 minecraft:stone')
    command('fill -3 65 -3 3 70 4 minecraft:air')
    command('tp ClimbingProbe .5 65 .5 0 0')
    assert 'error' not in request('prepare', position=[.5, 65., .5])
    setblock('dirt')
    # Let the teleported player settle on the ground before the estimate.
    until(lambda: request('dig_estimate', pos=TARGET).get('ticks') == 15, 5)

    border(True)
    mark = trace.mark()
    rejected = request('strict_dig', pos=TARGET)
    assert 'error' not in rejected, rejected
    outcome = rejected['outcome']
    assert outcome['kind'] == 'rejected', rejected
    assert not rejected['removed'] and rejected['final_state']['name'] == 'minecraft:dirt', rejected
    evidence = outcome['rejection']
    assert evidence['target_receipt']['value']['name'] == 'minecraft:dirt', rejected
    if legacy:
        assert evidence['legacy_status'] == 0 and evidence['acknowledged_sequence'] is None, rejected
    else:
        assert evidence['acknowledged_sequence'] == rejected['sequences'][-1], rejected
        assert evidence['legacy_status'] is None, rejected
    assert target_is('dirt'), 'native target changed'
    report['checks'].append(dict(name='world_border_dig_rejected', record=rejected, frames=frames(mark)))

    border(False)
    mark = trace.mark()
    removed = request('strict_dig', pos=TARGET)
    assert 'error' not in removed, removed
    assert removed['outcome']['kind'] == 'removed' and removed['removed'], removed
    assert target_is('air'), 'native target not removed'
    report['checks'].append(dict(name='same_connection_dig_removed', record=removed, frames=frames(mark)))

    def empty_hand():
        # The removed block's drop may be picked up into the selected slot.
        command('kill @e[type=minecraft:item]')
        command('clear ClimbingProbe')
        until(lambda: request('player')['inventory']['slots'][36]['value']['kind'] == 'empty', 5)

    empty_hand()

    # Not every refused START is a no-op: the handler keeps its last destroy
    # position, and a STOP for it is not re-checked by mayInteract. The echo of
    # the refused START alone therefore never concludes rejection.
    setblock('dirt')
    border(True)
    mark = trace.mark()
    stale = request('strict_dig', pos=TARGET)
    assert 'error' not in stale, stale
    assert stale['outcome']['kind'] == 'removed' and stale['removed'], stale
    assert target_is('air'), 'native target not removed'
    report['checks'].append(dict(name='stale_destroy_position_stop_removes_inside_refusal',
                                 record=stale, frames=frames(mark)))
    border(False)
    empty_hand()

    if not legacy:
        # Common staged mining is checked last: a retained attempt keeps the
        # connection's one-shot owner, by design, even after it resolves.
        setblock('dirt', MINED)
        border(True)
        command('tp ClimbingProbe .5 65 .5 %s %s' % tuple(ROTATION))
        until(lambda: (p := request('player')).get('received_pose') and
              abs(p['received_pose']['rotation'][1] - ROTATION[1]) < 0.01, 5)
        request('wait', ms=200)
        mark = trace.mark()
        started = request('mine_start', pos=MINED)
        assert 'error' not in started and started['stage'] == 'mining', started
        request('wait', ms=started['estimated_wait_ms'])
        finish = request('mine_finish')
        assert 'error' not in finish, finish

        def resolved():
            record = request('mine_record')
            return record if record['stage'] not in ('mining', 'pending_after_finish') else None

        mined = until(resolved, 5)
        assert mined['stage'] == 'observed_rejected', mined
        assert mined['rejection']['acknowledged_sequence'] == mined['finish']['interaction_sequence'], mined
        assert mined['rejection']['boundary_receive_sequence'] == mined['protocol']['receive_sequence'], mined
        assert mined['rejection']['target_receipt']['state']['name'] == 'minecraft:dirt', mined
        assert mined['requires_inspection'] is None, mined
        assert target_is('dirt', MINED), 'native target changed'
        report['checks'].append(dict(name='common_mining_observed_rejected', start=started,
                                     record=mined, frames=frames(mark)))
        border(False)
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
    sources = ['src/client/dig.rs', 'src/client/operational_dig.rs',
               'src/versions/java_1_16_1/client/operational_dig.rs',
               'src/versions/java_1_21_11/client/operations/operational_dig.rs',
               'src/versions/java_1_21_11/client/operations/mining.rs',
               'examples/body_probe.rs', 'scripts/run_dig_rejection_live.py', 'Cargo.lock']
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
