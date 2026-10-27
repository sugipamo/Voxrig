#!/usr/bin/env python3
"""Verify 1.16.1 digging ACK corrections against original official-server packets."""
import argparse
import hashlib
import json
from pathlib import Path
import struct
import sys

sys.dont_write_bytecode = True
import run_climbing_control as harness
from run_common_native import PacketTraceProxy, until


def decode(frame):
    body = bytes.fromhex(frame['body_hex'])
    packed, = struct.unpack_from('>Q', body)
    def signed(n, bits):
        return n - (1 << bits) if n & (1 << (bits - 1)) else n
    position = [signed(packed >> 38, 26), signed(packed & 4095, 12),
                signed((packed >> 12) & 0x3ffffff, 26)]
    state, size = PacketTraceProxy.varint(body[8:])
    status, width = PacketTraceProxy.varint(body[8 + size:])
    assert len(body) == 9 + size + width
    assert body[-1] in (0, 1)
    return dict(position=position, block_state_id=state, status=status,
                successful=bool(body[-1]))


def check(version, command, request, trace, report):
    assert version == '1.16.1'
    command('tp ClimbingProbe 0.5 65 0.5')
    near, far = [0, 65, 2], [0, 65, 12]
    for name, mode, position, status, accepted, resulting_block in [
        ('survival_start_non_air', 'survival', near, 0, True, 'stone'),
        ('distant_start_rejected', 'survival', far, 0, False, 'stone'),
        ('distant_finish_rejected', 'survival', far, 2, False, 'stone'),
        ('creative_start_removed', 'creative', near, 0, True, 'air'),
    ]:
        command(f'gamemode {mode} ClimbingProbe')
        x, y, z = position
        command(f'setblock {x} {y} {z} minecraft:stone')
        until(lambda: request('blocks', positions=[position])['states'] == [1], 10)
        before = request('events')['cursor']
        mark = trace.mark()
        result = request('dig', position=position, status=status)
        assert 'error' not in result, result
        def observed():
            log = request('events', cursor=before)
            matches = [e for e in log['events'] if isinstance(e['kind'], dict)
                       and 'LegacyDiggingAcknowledged' in e['kind']]
            return (log, matches) if matches else None
        log, notifications = until(observed, 10)
        frames = until(lambda: [f for f in trace.since(mark) if f['phase'] == 'play'
                               and f['direction'] == 'clientbound' and f['packet_id'] == 0x07], 10)
        assert len(frames) == len(notifications) == 1, (frames, notifications)
        original = decode(frames[0])
        assert notifications[0]['kind']['LegacyDiggingAcknowledged'] == original
        assert original['position'] == position and original['status'] == status
        assert original['successful'] == accepted, original
        seq = notifications[0]['receive_sequence']
        same_packet = [e['kind'] for e in log['events'] if e['receive_sequence'] == seq]
        assert same_packet == [dict(BlocksChanged=dict(min=position, max=position)),
                               dict(LegacyDiggingAcknowledged=original)], same_packet
        states = request('blocks', positions=[position])['states']
        assert states == [original['block_state_id']], (states, original)
        assert 'Test passed' in command(f'execute if block {x} {y} {z} minecraft:{resulting_block}')
        if status == 2:
            assert result['outcome'] == 'Rejected', result
        report['checks'].append(dict(name=name, original=frames, decoded=original,
                                     result=result, notifications=log, states=states))
    trace.expect_disconnect()
    closed = request('disconnect')
    assert any(e['kind'] == 'Disconnected' for e in closed['events'])
    assert sum(isinstance(e['kind'], dict) and 'LegacyDiggingAcknowledged' in e['kind']
               for e in closed['events']) == 4
    report['checks'].append(dict(name='results_survive_disconnect', events=closed))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--accept-eula', action='store_true', required=True)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--jars', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    harness.ROOT = args.output.resolve()
    def proxy(port, version, path):
        return PacketTraceProxy(port, version, path, body_capture_filter=lambda f:
                                f['phase'] == 'play' and
                                ((f['direction'] == 'clientbound' and f['packet_id'] == 0x07)
                                 or (f['direction'] == 'serverbound' and f['packet_id'] == 0x1b)))
    harness.run('1.16.1', args.binary.resolve(), args.jars.resolve(), check=check, trace_factory=proxy)
    report_path = sorted(harness.ROOT.glob('*/report.json'))[-1]
    report = json.loads(report_path.read_text())
    repo = Path(__file__).resolve().parents[1]
    sources = ['src/client/events.rs', 'src/versions/java_1_16_1/client.rs',
               'src/versions/java_1_16_1/client/play_world.rs',
               'examples/digging_ack_probe.rs', 'scripts/run_digging_ack_live.py', 'Cargo.lock']
    report['source_sha256'] = {p: hashlib.sha256((repo / p).read_bytes()).hexdigest() for p in sources}
    (harness.ROOT / 'report.json').write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
