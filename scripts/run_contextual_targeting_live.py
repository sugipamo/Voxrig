#!/usr/bin/env python3
"""Compare read-only contextual targets with original dimension/clip fixtures.

Runs a disposable official 1.21.11 server copy and retains original packet/RCON
observations. The existing harness stops every process it starts.
"""
import argparse
import gzip
import hashlib
import json
import shutil
import tempfile
from pathlib import Path
import run_climbing_control as live
from run_common_native import until


def check(version, command, request, trace, report):
    assert version == '1.21.11'
    sources = ['Cargo.lock', 'examples/climbing_control_probe.rs',
               'src/client/adapter.rs', 'src/client/operations.rs', 'src/client/survival/target.rs',
               'src/versions/java_1_21_11/client/operations/movement.rs',
               'src/versions/java_1_21_11/client/operations/movement/contextual.rs',
               'src/versions/java_1_21_11/client/raycast.rs',
               'src/versions/java_1_21_11/client/raycast/outline.rs',
               'data/client_api/contextual_targeting.json.gz',
               'scripts/run_contextual_targeting_live.py', 'scripts/run_climbing_control.py',
               'scripts/run_common_native.py']
    report['source_sha256'] = {p: hashlib.sha256((live.REPO / p).read_bytes()).hexdigest() for p in sources}
    fixture = json.loads(gzip.decompress((live.REPO / 'data/client_api/contextual_targeting.json.gz').read_bytes()))

    def query(mode='survival'):
        return request('contextual_target', mode=mode)

    def attribute(name, value):
        boundary = request('player')['receive_sequence']
        command(f'attribute ClimbingProbe minecraft:{name} base set {value}')
        return until(lambda: a if (a := request('player')['attributes'].get('minecraft:' + name))
                     and a['value']['value'] == value and a['source']['sequence'] > boundary else None, 5)

    def original(sequence):
        return until(lambda: frames[sequence-1] if len(frames := [f for f in trace.since(0)
                     if f['direction'] == 'clientbound' and f['phase'] in ('configuration', 'play')]) >= sequence else None, 5)

    def reset():
        command('tp ClimbingProbe 0.5 65 0.5 0 90')
        request('prepare', position=[.5, 65., .5])

    def inspect(name, pose, scale, reach, mode='survival'):
        expected = next(c for c in fixture if c['pose'] == pose and c['scale'] == scale
                        and c['reach'] == reach and c['rotation'] == [0, 90] and c['scene'] == 0)
        before = request('player')
        native_before = command('data get entity ClimbingProbe Pos')
        boundary = trace.mark()
        result = query(mode)
        assert 'observation' in result, result
        observation = result['observation']
        assert observation['pose'] == ('standing' if pose == 0 else 'crouching'), observation
        assert observation['scale']['value'] == scale and observation['reach']['value'] == reach, observation
        assert observation['eye']['value'] == [expected['eye'][0], expected['eye'][1] - 116., expected['eye'][2]], observation
        assert observation['replaceable'] is None and observation['operation_permitted'] is None
        hit, want = observation['visibility']['value'], expected['hit']
        assert (hit is None) == (want is None), (hit, want)
        if hit:
            assert hit['position'] == [want['position'][0], want['position'][1] - 116, want['position'][2]], hit
            assert hit['face'].lower() == want['face'], hit
            assert max(abs(a - b) for a, b in zip(hit['point'], [want['point'][0], want['point'][1] - 116., want['point'][2]])) < 1e-10
            assert abs(hit['distance'] - sum((a-b)**2 for a,b in zip(expected['eye'], want['point']))**.5) < 1e-10
        after = request('player')
        assert before['position'] == after['position'] and before['rotation'] == after['rotation']
        assert native_before == command('data get entity ClimbingProbe Pos')
        # Keep-alive replies are transport maintenance, not query sends.
        outbound = [f for f in trace.since(boundary) if f['direction'] == 'serverbound' and f['phase'] == 'play' and f['packet_id'] != 0x1b]
        assert not outbound, outbound
        sources = {}
        for field in ('scale', 'reach'):
            packet = original(observation[field]['source']['sequence'])
            assert packet['packet_id'] == 0x81
            sources[field] = packet
        if observation['pose_source']['kind'] == 'received':
            sources['pose'] = original(observation['pose_source']['sequence'])
            assert sources['pose']['packet_id'] == 0x61
        report['checks'].append(dict(name=name, observation=observation, original_sources=sources,
                                     native_position=native_before, query_packets=outbound))

    reset()
    # Default values that were never transmitted must not be invented by the query.
    missing = query()
    if 'error' in missing:
        report['checks'].append(dict(name='missing_attributes_rejected', result=missing))
    for scale in (.125, .5, 1., 1.3, 2., 4.):
        attribute('scale', scale)
        reset()
        for reach in (0., .5, 4.5, 8.):
            attribute('block_interaction_range', reach)
            inspect(f'standing_scale_{scale}_reach_{reach}', 0, scale, reach)
    # Force native crouching with a low full-cube ceiling at scale 2, then stop
    # the controller and obtain a fresh stationary position packet. The server
    # keeps crouching after input release because standing no longer fits.
    attribute('scale', 2.)
    attribute('block_interaction_range', 4.5)
    command('fill -4 65 -4 4 84 4 minecraft:air')
    reset()
    owner = request('mode_start', mode='survival')
    assert owner['admitted'], owner
    owner = owner['record']
    keys = dict(forward=0, strafe=0, jump=False, sneak=True, sprint=False, yaw=0., pitch=90.)
    assert request('mode_keys', mode='survival', session_id=owner['session_id'], controls=keys)['admitted']
    request('ticks', count=4)
    command('fill -2 68 -2 2 68 2 minecraft:stone')
    request('ticks', count=3)
    assert request('mode_stop', mode='survival', session_id=owner['session_id'])['admitted']
    reset()
    until(lambda: query().get('observation', {}).get('pose') == 'crouching', 5)
    inspect('stationary_forced_crouch_scale_2', 5, 2., 4.5)
    command('fill -2 68 -2 2 68 2 minecraft:air')
    reset()
    until(lambda: query().get('observation', {}).get('pose') == 'standing', 5)
    command('gamemode creative ClimbingProbe')
    until(lambda: request('player')['game_mode'] == 'creative', 5)
    received = request('player')['attributes']['minecraft:block_interaction_range']['value']
    # Keep the actual creative modifier and choose a base yielding this fixture's reach.
    command(f"attribute ClimbingProbe minecraft:block_interaction_range base set {4.5 - received['value'] + received['base']}")
    until(lambda: request('player')['attributes']['minecraft:block_interaction_range']['value']['value'] == 4.5, 5)
    inspect('creative_ground_received_reach', 0, 2., 4.5, 'creative')
    trace.expect_disconnect()
    request('disconnect')


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary', type=Path, required=True)
    p.add_argument('--jar', type=Path, required=True)
    p.add_argument('--work', type=Path, required=True)
    p.add_argument('--accept-eula', action='store_true', required=True)
    a = p.parse_args()
    with tempfile.TemporaryDirectory(prefix='voxrig-contextual-') as temporary:
        jars = Path(temporary)
        shutil.copyfile(a.jar.resolve(), jars / '1.21.11-server.jar')
        live.ROOT = a.work.resolve()
        binary = a.binary.resolve()
        before = hashlib.sha256(binary.read_bytes()).hexdigest()
        live.run('1.21.11', binary, jars, check=check)
        assert hashlib.sha256(binary.read_bytes()).hexdigest() == before
        print(json.dumps(dict(binary_sha256=before)))


if __name__ == '__main__':
    main()
