#!/usr/bin/env python3
"""Compare 1.16.1 explosion block invalidation/history with original vanilla packets."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import struct
import sys

sys.dont_write_bytecode = True
import run_climbing_control as harness
from run_common_native import PacketTraceProxy, until


def decode(frame):
    body = bytes.fromhex(frame['body_hex'])
    x, y, z, radius, count = struct.unpack_from('>ffffi', body)
    assert len(body) == 32 + 3 * count, frame
    origin = [math.floor(v) for v in (x, y, z)]
    positions = [[origin[a] + offset[a] for a in range(3)]
                 for offset in struct.iter_unpack('bbb', body[20:20 + count * 3])]
    motion = list(struct.unpack_from('>fff', body, 20 + count * 3))
    return dict(center=[x, y, z], radius=radius, block_count=count,
                player_knockback=motion, presentation={'Java1_16_1': {'affected_blocks': positions}})


def check(version, command, request, trace, report):
    assert version == '1.16.1'
    command('gamemode creative ClimbingProbe')
    command('gamerule mobGriefing true')
    command('fill -4 64 -4 4 67 4 minecraft:stone')
    command('tp ClimbingProbe 0.5 70 0.5')
    # Keep the original floor snapshot as an upper consumer's old assumption.
    until(lambda: request('blocks', positions=[[0, 64, 0]])['states'] == [1], 10)
    before = request('events')['cursor']
    after = request('explosions')['receive_sequence']
    mark = trace.mark()
    command('summon minecraft:tnt 0.5 68 0.5 {Fuse:1s,NoGravity:1b}')
    history = until(lambda: (lambda h: h if h.get('explosions') else None)(
        request('explosions', after=after)), 10)
    frames = until(lambda: [f for f in trace.since(mark) if f['phase'] == 'play'
                           and f['direction'] == 'clientbound' and f['packet_id'] == 0x1c], 10)
    assert len(frames) == len(history['explosions']) == 1, (frames, history)
    received = history['explosions'][0]
    expected = decode(frames[0])
    assert received['explosion'] == expected, (received, expected)
    positions = expected['presentation']['Java1_16_1']['affected_blocks']
    assert positions and any(-4 <= p[0] <= 4 and 64 <= p[1] <= 67 and -4 <= p[2] <= 4
                             for p in positions), positions
    events = request('events', cursor=before)
    same_packet = [e for e in events['events'] if e['receive_sequence'] == received['receive_sequence']]
    bounds = dict(min=[min(p[a] for p in positions) for a in range(3)],
                  max=[max(p[a] for p in positions) for a in range(3)])
    assert [e['kind'] for e in same_packet] == [{'BlocksChanged': bounds}, 'ExplosionReceived'], same_packet
    states = request('blocks', positions=positions)['states']
    assert states == [0] * len(positions), states
    # RCON independently confirms the server removed the received positions.
    for x, y, z in positions:
        assert 'Test passed' in command(f'execute if block {x} {y} {z} minecraft:air')
    assert request('explosions', after=after)['explosions'] == history['explosions']
    report['checks'].append(dict(name='original_payload_common_invalidation_and_server_air',
                                 original=frames, history=history, notifications=same_packet,
                                 states=states))
    # A real empty-list explosion still has context, but invalidates no terrain.
    command('gamerule mobGriefing false')
    command('difficulty normal')
    before = request('events')['cursor']
    after = request('explosions')['receive_sequence']
    mark = trace.mark()
    command('summon minecraft:creeper 0.5 69 0.5 {Fuse:1s,ignited:1b,NoAI:1b}')
    empty = until(lambda: (lambda h: h if h.get('explosions') else None)(request('explosions', after=after)), 10)
    frame = until(lambda: [f for f in trace.since(mark) if f['phase'] == 'play'
                          and f['direction'] == 'clientbound' and f['packet_id'] == 0x1c], 10)
    received = empty['explosions'][0]
    assert received['explosion'] == decode(frame[0])
    assert received['explosion']['block_count'] == 0
    same_packet = [e for e in request('events', cursor=before)['events']
                   if e['receive_sequence'] == received['receive_sequence']]
    assert [e['kind'] for e in same_packet] == ['ExplosionReceived'], same_packet
    report['checks'].append(dict(name='empty_list_context_without_false_block_change',
                                 original=frame, history=empty, notifications=same_packet))
    trace.expect_disconnect()
    closed = request('disconnect')
    assert closed['explosions'] == history['explosions'] + empty['explosions'], closed
    report['checks'].append(dict(name='history_survives_disconnect', history=closed))


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
                                f['direction'] == 'clientbound' and f['phase'] == 'play'
                                and f['packet_id'] == 0x1c)
    harness.run('1.16.1', args.binary.resolve(), args.jars.resolve(), check=check, trace_factory=proxy)
    report_path = sorted(harness.ROOT.glob('*/report.json'))[-1]
    report = json.loads(report_path.read_text())
    repo = Path(__file__).resolve().parents[1]
    sources = ['src/client/explosions.rs', 'src/versions/java_1_16_1/client.rs',
               'src/versions/java_1_16_1/client/play_world.rs',
               'examples/explosion_changes_probe.rs', 'scripts/run_explosion_changes_live.py', 'Cargo.lock']
    report['source_sha256'] = {p: hashlib.sha256((repo / p).read_bytes()).hexdigest() for p in sources}
    (harness.ROOT / 'report.json').write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
