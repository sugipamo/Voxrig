#!/usr/bin/env python3
"""Own velocity override (knockback) on SHA-1 verified official Java 1.16.1 / 1.21.11 servers.

Only original server packets and RCON commands; no synthetic replies/cache edits.
The server sends the own player's velocity (1.16.1 0x46, 1.21.11 ENTITY_VELOCITY)
after it was hurt: once from an instant-damage effect and once from a zombie hit
(horizontal knockback). The clientbound frames recorded by the proxy are the
independent oracle: the received player snapshot, entity history and change
events must report the decoded value with that packet's receive sequence.
"""
import argparse
import hashlib
import json
from pathlib import Path
import struct
import sys

sys.dont_write_bytecode = True
import run_climbing_control as harness
from run_common_native import until

PACKETS = {'1.16.1': dict(join=0x25, velocity=0x46), '1.21.11': dict(join=0x30, velocity=0x63)}


def varint(data, offset=0):
    value = shift = 0
    while True:
        byte = data[offset]
        offset += 1
        value |= (byte & 0x7f) << shift
        if not byte & 0x80:
            return value - (1 << 32) if value >= 1 << 31 else value, offset
        shift += 7


def velocity(version, body):
    """Decode the original packet exactly like the native codec."""
    entity, offset = varint(body)
    if version == '1.16.1':
        return entity, [v / 8000.0 for v in struct.unpack('>hhh', body[offset:offset + 6])]
    first = body[offset]
    if first == 0:
        return entity, [0.0, 0.0, 0.0]
    second = body[offset + 1]
    packed = (struct.unpack('>I', body[offset + 2:offset + 6])[0] << 16) | (second << 8) | first
    scale = first & 3
    if first & 4:
        extra, _ = varint(body, offset + 6)
        scale |= (extra & 0xffffffff) << 2
    return entity, [(min((packed >> (3 + axis * 15)) & 32767, 32766) * 2.0 / 32766.0 - 1.0) * scale
                    for axis in range(3)]


def check(version, command, request, trace, report):
    ids = PACKETS[version]

    def play(frames, packet):
        return [f for f in frames if f['phase'] == 'play' and f['direction'] == 'clientbound'
                and f['packet_id'] == packet]

    join = play(trace.since(0), ids['join'])
    assert len(join) == 1, join
    own = struct.unpack('>i', bytes.fromhex(join[0]['body_hex'])[:4])[0]
    command('tp ClimbingProbe .5 65 .5 0 0')
    request('prepare', position=[.5, 65., .5])
    request('wait', ms=4000)  # native 60-tick spawn invulnerability after join
    before = request('own_velocity')

    def received(name, provoke, accept):
        mark = trace.mark()
        provoke()

        def arrived():
            frames = [f for f in play(trace.since(mark), ids['velocity'])
                      if velocity(version, bytes.fromhex(f['body_hex']))[0] == own]
            return frames if any(accept(velocity(version, bytes.fromhex(f['body_hex']))[1])
                                 for f in frames) else None
        until(arrived, 20)
        request('wait', ms=600)  # let later own velocity packets of this hurt arrive
        frames = [f for f in play(trace.since(mark), ids['velocity'])
                  if velocity(version, bytes.fromhex(f['body_hex']))[0] == own]
        expected = velocity(version, bytes.fromhex(frames[-1]['body_hex']))[1]
        state = request('own_velocity')
        field = state['velocity_override']
        assert isinstance(field, dict), state
        assert field['value'] == expected, (field, expected, frames)
        sequence = field['ordinal']
        history = [r for r in state['history'] if r['receive_sequence'] == sequence]
        assert [r['kind']['OwnVelocity']['velocity'] for r in history] == [expected], state['history']
        # Every own velocity frame since the mark has exactly one history record.
        recorded = [r for r in state['history'] if r['ordinal'] not in
                    {h['ordinal'] for h in before_history}]
        assert [r['kind']['OwnVelocity']['velocity'] for r in recorded] == \
            [velocity(version, bytes.fromhex(f['body_hex']))[1] for f in frames], (recorded, frames)
        kinds = [e['kind'] for e in state['events'] if e['receive_sequence'] == sequence]
        assert 'PlayerChanged' in kinds, kinds
        updated = {'EntityUpdated': {'native_id': own}}
        # 1.21.11 keeps EntityUpdated{self} for compatibility; the legacy client
        # does not cache its own entity and reports only PlayerChanged.
        assert (updated in kinds) == (version == '1.21.11'), kinds
        report['checks'].append(dict(name=name, own_entity_id=own, frames=frames, expected=expected,
                                     velocity_override=field, history=history, kinds=kinds,
                                     player=state['player']))
        before_history.extend(recorded)
        return field

    before_history = list(before['history'])
    assert before['velocity_override'] == 'missing', before
    first = received('instant_damage_velocity',
                     lambda: command('effect give ClimbingProbe minecraft:instant_damage 1 0'),
                     lambda v: True)
    command('difficulty easy')
    command('time set night')
    command('summon minecraft:zombie 2.0 65 .5 {PersistenceRequired:1b,IsBaby:0b}')
    second = received('zombie_knockback_velocity', lambda: None,
                      lambda v: abs(v[0]) > 0.05 or abs(v[2]) > 0.05)
    command('kill @e[type=minecraft:zombie]')
    command('difficulty peaceful')
    assert second['ordinal'] > first['ordinal'], (first, second)
    assert abs(second['value'][0]) > 0.05 or abs(second['value'][2]) > 0.05, second
    trace.expect_disconnect()
    request('disconnect')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--accept-eula', action='store_true', required=True)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--jars', type=Path, required=True)
    parser.add_argument('--version', choices=sorted(PACKETS), required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    harness.ROOT = args.output.resolve()
    harness.run(args.version, args.binary.resolve(), args.jars.resolve(), check=check)
    report_path = sorted(harness.ROOT.glob('*/report.json'))[-1]
    report = json.loads(report_path.read_text())
    sources = ['src/client/received_snapshot.rs', 'src/client/entity/history.rs',
               'src/versions/java_1_16_1/client/play_entities.rs',
               'src/versions/java_1_16_1/client/received_snapshot.rs',
               'src/versions/java_1_21_11/client.rs',
               'src/versions/java_1_21_11/client/operations/survival.rs',
               'src/versions/java_1_21_11/client/received_snapshot.rs',
               'examples/body_probe.rs', 'scripts/run_own_velocity_live.py', 'Cargo.lock']
    report['binary_sha256'] = hashlib.sha256(args.binary.read_bytes()).hexdigest()
    report['source_sha256'] = {p: hashlib.sha256((harness.REPO / p).read_bytes()).hexdigest() for p in sources}
    (harness.ROOT / 'report.json').write_text(json.dumps(report, ensure_ascii=False, indent=2) + '\n')
    if not report['passed']:
        raise SystemExit(1)
