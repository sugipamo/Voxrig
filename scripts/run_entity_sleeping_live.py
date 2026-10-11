#!/usr/bin/env python3
"""Qualify received sleeping positions and frozen deltas on official servers."""
import argparse
import functools
import hashlib
from pathlib import Path
import subprocess

from run_climbing_control import REPO, run
from run_common_native import PacketTraceProxy, until
from run_entity_data_history_live import check as history_check


def sleeping_checks(version, command, request, trace, report, identity, cow, drain):
    original = next(e for e in request('entity_sleeping') if e['entity']['id'] == identity)
    if version == '1.21.11':
        assert original['sleeping'] is None  # Original modern server omits the default.
    else:
        assert original['sleeping'] is not None and not original['sleeping']['sleeping']
    report['checks'].append(dict(name='sleeping_initial_received_or_missing', observation=original))
    last_position = None
    for number, position in enumerate([[-2, 65, 5], [2, 65, -3], None, [-1, 65, 4], None]):
        if position is None:
            # Removing the actual bed runs the original LivingEntity wake path
            # and sends the original explicit empty optional metadata.
            command('setblock %d %d %d minecraft:air' % tuple(last_position))
        else:
            command('setblock %d %d %d minecraft:red_bed[part=head,facing=south]' % tuple(position))
            foot = [position[0], position[1], position[2] - 1]
            command('setblock %d %d %d minecraft:red_bed[part=foot,facing=south]' % tuple(foot))
            if version == '1.16.1':
                command('data merge entity @e[tag=DataHistoryCow,limit=1] {SleepingX:%d, SleepingY:%d, SleepingZ:%d}' % tuple(position))
            else:
                command('data merge entity @e[tag=DataHistoryCow,limit=1] {sleeping_pos:[I;%d,%d,%d]}' % tuple(position))
            last_position = position
        expected = None if position is None else dict(zip(['x', 'y', 'z'], position))
        def observed():
            reading = next((e for e in request('entity_sleeping') if e['entity']['id'] == identity), None)
            return reading if reading and reading['sleeping'] is not None and reading['sleeping']['position'] == expected else None
        result = until(observed, 10)
        value = result['sleeping']
        assert value['index'] == (13 if version == '1.16.1' else 14)
        assert value['sleeping'] == (position is not None)
        assert 'Received' in value['source'] and value['metadata_complete']
        index = str(value['index'])
        current = cow()['metadata'][index]
        assert current['value'] == {'OptionalBlockPosition': expected}
        assert current['source'] == dict(kind='received', sequence=value['source']['Received']['sequence'])
        drain()
        report['checks'].append(dict(name='sleeping_position_' + str(number), observation=result))
    # Legacy Turtle still sends plain BlockPos metadata; modern Turtle moved
    # this field out of metadata. Do not manufacture a modern equivalent.
    if version == '1.16.1':
        command('summon minecraft:turtle 3.5 65 5.5 {Tags:["PositionTurtle"],NoAI:1b,NoGravity:1b,HomePosX:-99999,HomePosY:-64,HomePosZ:88888}')
        def received_turtle():
            for entity in request('entities')['entities']:
                if entity['motion']['entity']['type_name'] == 'minecraft:turtle':
                    if any(v['value'] == {'BlockPosition': dict(x=-99999, y=-64, z=88888)} for v in entity['metadata'].values()):
                        return entity
        turtle = until(received_turtle, 10)
        drain()
        report['checks'].append(dict(name='legacy_original_turtle_plain_position', identity=turtle['motion']['entity']['id']))
        command('kill @e[tag=PositionTurtle]')
        drain()


def check(version, command, request, trace, report, binary, sdk):
    history_check(version, command, request, trace, report, binary, sdk, additional_checks=sleeping_checks)


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--accept-eula', action='store_true', required=True)
    p.add_argument('--binary', type=Path, required=True)
    p.add_argument('--jars', type=Path, default=REPO / '.local/climbing/downloads')
    p.add_argument('--version', choices=['1.16.1', '1.21.11'], action='append')
    p.add_argument('--compiled-sdk-revision', required=True)
    args = p.parse_args()
    revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], text=True, cwd=REPO).strip()
    compiled = subprocess.check_output(['git', 'rev-parse', args.compiled_sdk_revision], text=True, cwd=REPO).strip()
    paths = ['src', 'examples', 'Cargo.toml', 'Cargo.lock', 'data']
    assert not subprocess.check_output(['git', 'diff', compiled, 'HEAD', '--', *paths], cwd=REPO)
    assert not subprocess.check_output(['git', 'diff', 'HEAD', '--', *paths], cwd=REPO)
    artifacts = ['Cargo.lock', 'scripts/run_entity_sleeping_live.py', 'scripts/run_entity_data_history_live.py',
                 'scripts/run_entity_pose_live.py', 'scripts/run_climbing_control.py', 'scripts/run_common_native.py',
                 'examples/climbing_control_probe.rs', 'data/client_api/entity_position_oracle.json',
                 'scripts/EntityPositionOracle.java', 'scripts/run_entity_position_oracle.py']
    sdk = dict(source_revision=revision, binary_build_revision=compiled,
               binary_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest(),
               artifacts={f: hashlib.sha256((REPO / f).read_bytes()).hexdigest() for f in artifacts})
    for version in args.version or ['1.16.1', '1.21.11']:
        run(version, args.binary.resolve(), args.jars.resolve(), check=functools.partial(check, binary=args.binary.resolve(), sdk=sdk),
            trace_factory=functools.partial(PacketTraceProxy, body_capture_filter=lambda frame: frame['direction'] == 'clientbound' and frame['phase'] == 'play' and frame['packet_id'] in (0x44, 0x47, 0x61, 0x64, 0x81) and frame['body_length'] <= 65536))


if __name__ == '__main__':
    main()
