#!/usr/bin/env python3
"""Read received-target entity rays on an isolated official 1.21.11 server.

Packet traces establish input provenance and absence of query sends. RCON checks
fixture placement/unchanged positions; native clip arithmetic is a separate fixture.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import queue
import re
import shutil
import subprocess
import tempfile
import threading
import run_climbing_control as live
from run_common_native import until


def check(version, command, request, trace, report):
    assert version == '1.21.11'
    sources = ['Cargo.lock', 'Cargo.toml', 'examples/climbing_control_probe.rs',
               'src/client/entity/targeting.rs', 'src/client/entity/query.rs',
               'src/client/vehicle.rs', 'src/client/adapter.rs', 'src/client/operations.rs',
               'src/versions/java_1_21_11/client/operations/movement/contextual.rs',
               'src/versions/java_1_21_11/client/raycast.rs',
               'scripts/run_entity_target_live.py', 'scripts/run_climbing_control.py',
               'scripts/run_common_native.py', 'data/client_api/entity_target_rays.json.gz']
    report['source_sha256'] = {p: hashlib.sha256((live.REPO / p).read_bytes()).hexdigest() for p in sources}
    report['binary_sha256'] = hashlib.sha256(BINARY.read_bytes()).hexdigest()
    command('difficulty normal')
    command('gamerule minecraft:mob_drops false')
    command('kill @e[type=!minecraft:player]')
    command('tp ClimbingProbe 0.5 65 0.5 0 0')
    request('prepare', position=[.5, 65., .5])
    command('attribute ClimbingProbe minecraft:scale base set 1.3')
    command('attribute ClimbingProbe minecraft:scale base set 1')
    command('attribute ClimbingProbe minecraft:entity_interaction_range base set 6')
    until(lambda: request('player')['attributes'].get('minecraft:entity_interaction_range', {}).get('value', {}).get('value') == 6., 5)

    def query():
        return request('entity_target')

    def inspect(name, target_type, occluded):
        def complete():
            value = query()
            if 'observation' not in value:
                return None
            observation = value['observation']
            if not observation['candidates']:
                return None
            return observation
        observation = until(complete, 8)
        entities = request('entities')['entities']
        expected = [e for e in entities if e['motion']['entity']['type_name'] == target_type]
        assert len(expected) == 1, expected
        candidate = next(c for c in observation['candidates'] if c['entity'] == expected[0]['motion']['entity']['id'])
        assert candidate['block_occluded'] == occluded, candidate
        assert candidate['pick_radius'] == 0.
        assert observation['reach']['value'] == 6.
        assert observation['block_hit'] is not None, observation
        assert observation['block_hit']['position'] == [0, 66, 3], observation
        originals = [f for f in trace.since(0) if f['connection'] == 1 and f['direction'] == 'clientbound'
                     and f['phase'] in ('configuration', 'play')]
        source_packets = {}
        for field, source, packet_id in (
            ('own_scale', observation['scale']['source'], 0x81),
            ('entity_range', observation['reach']['source'], 0x81),
            ('remote_scale', candidate['geometry']['evidence']['scale']['attribute']['source'], 0x81),
        ):
            assert source['kind'] == 'received', source
            packet = originals[source['sequence'] - 1]
            assert packet['packet_id'] == packet_id, packet
            source_packets[field] = packet
        selector = 'RayPeer' if target_type == 'minecraft:player' else '@e[tag=RayTarget,limit=1]'
        native_target = command('data get entity ' + selector + ' Pos')
        position = re.search(r"\[([^]]+)\]", native_target)
        assert position, native_target
        actual = [float(v.strip().rstrip('df')) for v in position[1].split(',')]
        received = expected[0]['motion']['position']['value']['position']
        assert max(abs(a - b) for a, b in zip(actual, received)) < 1e-10, (actual, received)
        native_wall = command('execute if block 0 66 3 minecraft:stone')
        assert 'Test passed' in native_wall, native_wall
        native_before = command('data get entity ClimbingProbe Pos')
        mark = trace.mark()
        repeated = query()
        assert 'observation' in repeated, repeated
        frames = [f for f in trace.since(mark) if f['direction'] == 'serverbound'
                  and f['connection'] == 1 and f['phase'] == 'play' and f['packet_id'] != 0x1b]
        assert not frames, frames
        assert native_before == command('data get entity ClimbingProbe Pos')
        report['checks'].append(dict(name=name, observation=observation, native_position=native_before,
                                     entity_receipt=expected[0], native_target=native_target, native_wall=native_wall,
                                     original_sources=source_packets, query_packets=frames))

    for species in ('cow', 'pig', 'zombie'):
        for behind in (False, True):
            command('fill -1 65 3 1 68 3 minecraft:air')
            z = 4.5 if behind else 2.5
            command(f'summon minecraft:{species} 0.5 66 {z} {{Tags:["RayTarget"],NoAI:1b,NoGravity:1b,Silent:1b,PersistenceRequired:1b}}')
            # Modern servers omit unchanged scale defaults. Force an actual receipt.
            command('attribute @e[tag=RayTarget,limit=1] minecraft:scale base set 1.3')
            command('attribute @e[tag=RayTarget,limit=1] minecraft:scale base set 1')
            until(lambda: any(e['motion']['entity']['type_name'] == 'minecraft:' + species
                             and e['scale_attribute'] for e in request('entities')['entities']), 5)
            command('fill -1 65 3 1 68 3 minecraft:stone')
            request('wait', ms=250)
            inspect(species + ('_behind_wall' if behind else '_before_wall'), 'minecraft:' + species, behind)
            command('kill @e[tag=RayTarget]')
            until(lambda: not any(e['motion']['entity']['type_name'] == 'minecraft:' + species
                                 for e in request('entities')['entities']), 8)

    # A real second protocol connection supplies the player spawn/scale receipt.
    command('fill -1 65 3 1 68 3 minecraft:air')
    command('setworldspawn 6 65 6')
    peer = None
    try:
        env = dict(os.environ, VOXRIG_MINECRAFT_VERSION=version, VOXRIG_PORT=str(trace.port),
                   VOXRIG_PROBE_USERNAME='RayPeer')
        peer = subprocess.Popen([str(BINARY)], cwd=live.REPO, env=env, stdin=subprocess.PIPE,
                                stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True, bufsize=1)
        messages = queue.Queue()
        def read_peer():
            for line in peer.stdout:
                messages.put(line.rstrip())
        threading.Thread(target=read_peer, daemon=True).start()
        assert json.loads(messages.get(timeout=25)) == {'ready': True}
        command('attribute RayPeer minecraft:scale base set 1.3')
        command('attribute RayPeer minecraft:scale base set 1')
        for behind in (False, True):
            z = 4.5 if behind else 2.5
            command(f'tp RayPeer 0.5 65 {z} 0 0')
            command('fill -1 65 3 1 68 3 minecraft:stone')
            request('wait', ms=350)
            inspect('player_behind_wall' if behind else 'player_before_wall', 'minecraft:player', behind)
    finally:
        if peer and peer.poll() is None:
            trace.expect_disconnect()
            peer.stdin.write('{"command":"disconnect"}\n')
            peer.stdin.flush()
            try:
                peer.wait(timeout=10)
            except subprocess.TimeoutExpired:
                peer.kill()
                peer.wait(timeout=5)
    trace.expect_disconnect(connection=1)
    request('disconnect')


def main():
    global BINARY
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--binary', type=Path, required=True)
    p.add_argument('--jar', type=Path, required=True)
    p.add_argument('--work', type=Path, required=True)
    p.add_argument('--accept-eula', action='store_true', required=True)
    a = p.parse_args()
    BINARY = a.binary.resolve()
    with tempfile.TemporaryDirectory(prefix='voxrig-entity-ray-') as temporary:
        jars = Path(temporary)
        shutil.copyfile(a.jar.resolve(), jars / '1.21.11-server.jar')
        live.ROOT = jars / 'server'
        destination = a.work.resolve()
        destination.mkdir(parents=True, exist_ok=True)
        try:
            live.run('1.21.11', BINARY, jars, check=check)
        finally:
            for run in live.ROOT.glob('1.21.11-*'):
                retained = destination / run.name
                retained.mkdir(parents=True, exist_ok=True)
                for name in ('report.json', 'packets.jsonl', 'server.log', 'probe.log', 'probe.stderr.log'):
                    if (run / name).exists():
                        shutil.copyfile(run / name, retained / name)
        print(json.dumps({'retained': str(destination)}))


if __name__ == '__main__':
    main()
