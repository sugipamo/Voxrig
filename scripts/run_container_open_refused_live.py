#!/usr/bin/env python3
"""Refused container opening on SHA-1 verified official Java 1.16.1 and 1.21.11 servers.

A locked barrel and a chest under an occluding block make the original block
refuse to open a menu. Only original server packets and RCON commands; no
synthetic replies/cache edits. The same connection then opens an unlocked
barrel (success regression) after the terminal no-screen intents.
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
OPEN_ID = {'1.16.1': 0x2e, '1.21.11': 0x39}
ACK_ID = 0x04  # 1.21.11 BlockChangedAck; 1.16.1 has none
BLOCK_UPDATE = {'1.16.1': 0x0b, '1.21.11': 0x08}


def check(version, command, request, trace, report):
    def resolved():
        record = request('container_open_record')
        return record if record and record['stage'] not in ('pending', 'dispatched') else None

    def fixture(block, lock=False, above='air'):
        command('setblock %d %d %d minecraft:air' % tuple(TARGET))
        command('setblock %d %d %d minecraft:%s' % (TARGET[0], TARGET[1] + 1, TARGET[2], above))
        command('setblock %d %d %d minecraft:%s' % (*TARGET, block))
        if lock:
            nbt = '{Lock:"voxrig-key"}' if version == '1.16.1' else '{lock:{items:"minecraft:diamond"}}'
            command('data merge block %d %d %d %s' % (*TARGET, nbt))
            report.setdefault('lock', command('data get block %d %d %d' % tuple(TARGET)))
        request('wait', ms=400)

    def attempt(name, expected):
        mark = trace.mark()
        sent = request('strict_open', pos=TARGET)
        assert 'error' not in sent and sent['send']['dispatched'], sent
        record = until(resolved, 10)
        frames = trace.since(mark)
        clientbound = [f for f in frames if f['direction'] == 'clientbound' and f['phase'] == 'play']
        opens = [f for f in clientbound if f['packet_id'] == OPEN_ID[version]]
        assert record['id'] == sent['id'] and record['requires_inspection'] is None, record
        assert record['stage'] == expected, record
        if expected == 'no_screen_observed':
            assert not opens, opens
            assert record['observed_screen'] is None, record
            boundary = record['no_screen']['boundary_receive_sequence']
            if version == '1.16.1':
                assert record['protocol_processing'] is None, record
                assert record['no_screen']['clicked_echo_sequence'] + 1 == boundary, record
            else:
                ack = record['protocol_processing']
                assert ack['acknowledged_sequence'] >= record['send']['interaction_sequence'], record
                assert ack['receive_sequence'] == boundary, record
                assert record['no_screen']['clicked_echo_sequence'] is None, record
            assert request('screen').get('screen') is None
        else:
            assert len(opens) == 1 and record['no_screen'] is None, record
        order = [hex(f['packet_id']) for f in clientbound
                 if f['packet_id'] in (OPEN_ID[version], BLOCK_UPDATE[version])
                 or (version == '1.21.11' and f['packet_id'] == ACK_ID)]
        report['checks'].append(dict(name=name, sent=sent, record=record, packet_order=order, frames=frames))
        return record

    command('tp ClimbingProbe .5 65 .5 0 35')
    command('clear ClimbingProbe')
    request('prepare', position=[.5, 65., .5])

    fixture('barrel[facing=north,open=false]', lock=True)
    attempt('locked_barrel_no_screen', 'no_screen_observed')
    assert 'Test passed' in command('execute if block %d %d %d minecraft:barrel[open=false]' % tuple(TARGET))

    fixture('chest[facing=north,type=single,waterlogged=false]', above='stone')
    attempt('chest_under_occluding_block_no_screen', 'no_screen_observed')

    # Same connection: the released owner opens an unlocked barrel normally.
    fixture('barrel[facing=north,open=false]')
    attempt('unlocked_barrel_opens', 'observed_contents')
    until(lambda: 'Test passed' in command('execute if block %d %d %d minecraft:barrel[open=true]' % tuple(TARGET)) or None, 5)
    closed = request('close_screen')
    assert 'error' not in closed, closed
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
    sources = ['src/client/container/open.rs',
               'src/versions/java_1_16_1/client/common_container.rs',
               'src/versions/java_1_21_11/client/operations/container.rs',
               'examples/body_probe.rs', 'scripts/run_container_open_refused_live.py', 'Cargo.lock']
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
