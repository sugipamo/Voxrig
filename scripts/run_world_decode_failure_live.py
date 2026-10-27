#!/usr/bin/env python3
"""1.16.1 official-server reception plus explicitly labeled decode fault injection.

The unmodified official jar generates all traffic. After validating normal world
and light reception, a proxy truncates one selected chunk/light payload by one
byte and re-frames it with valid compression. This is transport fault injection,
not evidence that the vanilla server emits malformed packets.
"""
import argparse
import hashlib
import json
from pathlib import Path
import sys
import zlib

sys.dont_write_bytecode = True
import run_climbing_control as harness
from run_common_native import PacketTraceProxy, until


def varint(value):
    result = bytearray()
    while value > 127:
        result.append((value & 127) | 128)
        value >>= 7
    result.append(value)
    return bytes(result)


class DecodeFaultProxy(PacketTraceProxy):
    def __init__(self, *args):
        self.armed = None
        self.injection = None
        super().__init__(*args, body_capture_filter=lambda f:
                         f['phase'] == 'play' and f['direction'] == 'clientbound'
                         and f['packet_id'] in (0x21, 0x24))

    def forward_frame(self, record, destination, wire):
        with self.lock:
            inject = (self.armed == record['packet_id'] and record['phase'] == 'play'
                      and record['direction'] == 'clientbound')
            if inject:
                self.armed = None
                original = bytes.fromhex(record['body_hex'])
                body = original[:-1]
                payload = varint(record['packet_id']) + body
                state = next(s for s in self.connection_states
                             if s['connection'] == record['connection'])
                compression = state['compression']
                if compression is None:
                    frame = payload
                elif len(payload) >= compression:
                    frame = varint(len(payload)) + zlib.compress(payload)
                else:
                    frame = b'\0' + payload
                wire = varint(len(frame)) + frame
                self.injection = dict(original=record.copy(), delivered_body_hex=body.hex(),
                                      delivered_wire_sha256=hashlib.sha256(wire).hexdigest(),
                                      fault='drop final payload byte; valid framing/compression')
        destination.sendall(wire)


def check(version, command, request, trace, report):
    assert version == '1.16.1'
    baseline = until(lambda: (lambda o: o if o.get('state') == 1
                             and o.get('sky_light') is not None
                             and o.get('block_light') is not None else None)(request('observe')), 10)
    assert baseline['context_present'] and baseline['loaded'] > 0, baseline
    assert request('snapshot') == {'ok': True}
    normal = request('events')
    assert not any(e['kind'] in ('ObservationInvalidated', 'Disconnected') for e in normal['events'])
    originals = trace.since(0)
    assert any(f['phase'] == 'play' and f['direction'] == 'clientbound'
               and f['packet_id'] == 0x21 for f in originals)
    assert any(f['phase'] == 'play' and f['direction'] == 'clientbound'
               and f['packet_id'] == 0x24 for f in originals)
    report['checks'].append(dict(name='unmodified_official_chunk_and_light_reception',
                                 observation=baseline, event_cursor=normal['cursor']))
    cursor = normal['cursor']
    # Prepare a distant, real server column before moving the player. The view
    # transition supplies a fresh original chunk and light update for the proxy.
    command('forceload add 160 0 160 0')
    command('setblock 160 64 0 minecraft:stone')
    trace.expect_disconnect()  # The injected loss intentionally terminates the client.
    with trace.lock:
        trace.armed = report['fault_packet_id']
    command('tp ClimbingProbe 160.5 65 0.5')
    until(lambda: trace.injection, 15)
    def failed():
        log = request('events', cursor=cursor)
        return log if any(e['kind'] == 'Disconnected' for e in log['events']) else None
    log = until(failed, 10)
    invalid = [e for e in log['events'] if e['kind'] == 'ObservationInvalidated']
    terminal = [e for e in log['events'] if e['kind'] == 'Disconnected']
    assert len(invalid) == len(terminal) == 1, log
    assert log['events'][-2:] == invalid + terminal
    assert invalid[0]['receive_sequence'] == terminal[0]['receive_sequence'] == log['receive_sequence']
    snapshot = request('snapshot')
    assert f"play packet {report['fault_packet_id']:#04x}" in snapshot.get('error', ''), snapshot
    failures = {name: request(name) for name in ('observe', 'context', 'loaded')}
    # Wait from the final cursor: a drained terminal log must not look complete/live.
    failures['wait'] = request('wait', cursor=log['cursor'])
    assert all('error' in result for result in failures.values()), failures
    assert request('events', cursor=cursor) == log
    report['checks'].append(dict(name='fault_injection_invalidates_and_terminates',
                                 injection=trace.injection, notifications=log,
                                 snapshot_error=snapshot, subsequent_reads=failures))
    assert request('finish') == {'finished': True}
    report['authority_limits'] = ('Unmodified SHA-1 verified official 1.16.1 server and real TCP/RCON. '
        'Normal reception checked before proxy-only payload truncation. Malformed packets are '
        'explicit fault injection; no claim that the official server naturally sends them. '
        'Bounded-log eviction is tested by deterministic fixtures, not by this LIVE run.')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--accept-eula', action='store_true', required=True)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--jars', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    harness.ROOT = args.output.resolve()
    reports = []
    for packet_id in (0x21, 0x24):
        def scenario(version, command, request, trace, report):
            report['fault_packet_id'] = packet_id
            check(version, command, request, trace, report)
        harness.run('1.16.1', args.binary.resolve(), args.jars.resolve(), check=scenario,
                    trace_factory=DecodeFaultProxy)
        path = sorted(harness.ROOT.glob('*/report.json'))[-1]
        reports.append(json.loads(path.read_text()))
    repo = Path(__file__).resolve().parents[1]
    sources = ['src/client/events.rs', 'src/versions/java_1_16_1/client.rs',
               'src/versions/java_1_16_1/client/play_world.rs',
               'src/versions/java_1_16_1/client/common_chunks.rs',
               'examples/world_decode_failure_probe.rs', 'scripts/run_world_decode_failure_live.py',
               'Cargo.lock']
    evidence = dict(passed=all(r['passed'] for r in reports), runs=reports,
                    source_sha256={p: hashlib.sha256((repo / p).read_bytes()).hexdigest() for p in sources})
    (harness.ROOT / 'report.json').write_text(json.dumps(evidence, ensure_ascii=False, indent=2) + '\n')
