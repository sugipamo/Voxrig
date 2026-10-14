#!/usr/bin/env python3
"""Observe a second common client crouching on unchanged official servers."""
import argparse
import functools
import hashlib
import json
import os
from pathlib import Path
import queue
import subprocess
import threading
from run_climbing_control import REPO, run
from run_common_native import PacketTraceProxy, pump, until


def check(version, command, request, trace, report, binary, sdk, disconnect_observer=True):
    report['sdk'] = sdk
    folder = Path(trace.log.name).parent
    peer_trace = PacketTraceProxy(trace.server_port, version, folder / 'peer-packets.jsonl')
    peer = None
    try:
        with (folder / 'peer.log').open('w') as output, (folder / 'peer.stderr.log').open('w') as stderr:
            peer = subprocess.Popen([str(binary)], cwd=REPO, env=dict(os.environ,
                VOXRIG_MINECRAFT_VERSION=version, VOXRIG_PORT=str(peer_trace.port),
                VOXRIG_PROBE_USERNAME='PosePeer'), stdin=subprocess.PIPE,
                stdout=subprocess.PIPE, stderr=stderr, text=True, bufsize=1)
            messages = queue.Queue()
            reader = threading.Thread(target=pump, args=(peer.stdout, messages, output), daemon=True)
            reader.start()
            def receive():
                return json.loads(messages.get(timeout=20))
            assert receive() == {'ready': True}
            def ask(action, **arguments):
                peer.stdin.write(json.dumps(dict(command=action, **arguments)) + '\n')
                peer.stdin.flush()
                return receive()
            command('tp PosePeer 0.5 65 2.5 0 0')
            ask('prepare', position=[.5, 65., 2.5])
            def observed():
                rows = request('entity_poses')
                return rows[0] if len(rows) == 1 else None
            initial = until(observed, 10)
            identity = initial['entity']['id']
            report['checks'].append(dict(name='original_peer_spawn', observation=initial))
            owner = ask('mode_start', mode='survival')
            assert owner['admitted'], owner
            session = owner['record']['session_id']
            for name, crouching, native in [('crouching', True, 5), ('standing', False, 0)]:
                keys = dict(forward=0, strafe=0, jump=False, sneak=crouching,
                            sprint=False, yaw=0., pitch=0.)
                result = ask('mode_keys', mode='survival', session_id=session, controls=keys)
                assert result['admitted'], result
                def ready():
                    row = observed()
                    return row if row and row['pose'] and row['pose']['pose'] == name else None
                row = until(ready, 10)
                pose = row['pose']
                assert row['entity']['id'] == identity
                assert pose['native_id'] == native and pose['index'] == 6
                sequence = pose['source']['Received']['sequence']
                received = [p for p in trace.since(0) if p['direction'] == 'clientbound'
                            and p['phase'] in ('configuration', 'play')]
                original = received[sequence - 1]
                assert original['packet_id'] == (0x44 if version == '1.16.1' else 0x61)
                body = bytes.fromhex(original['body_hex'])
                entity_id, offset = PacketTraceProxy.varint(body)
                assert entity_id == identity['native_id']
                # Player flags precede pose; require the original accessor,
                # serializer, value and terminator in this actual frame.
                field = bytes([6, 18 if version == '1.16.1' else 20, native, 255])
                assert body[offset:].endswith(field), original
                report['checks'].append(dict(name='received_' + name, observation=row,
                    original_metadata_packet=original, peer_control=result))
            stopped = ask('mode_stop', mode='survival', session_id=session)
            assert stopped['admitted'], stopped
            peer_trace.expect_disconnect()
            ask('disconnect')
            assert peer.wait(timeout=10) == 0
            until(lambda: not request('entity_poses'), 10)
            report['checks'].append(dict(name='removed_peer_not_observed', removed_identity=identity))
            report['peer_trace_errors'] = peer_trace.errors
            assert not peer_trace.errors, peer_trace.errors
    finally:
        if peer and peer.poll() is None:
            peer.terminate()
            try:
                peer.wait(timeout=5)
            except subprocess.TimeoutExpired:
                peer.kill()
                peer.wait(timeout=5)
        peer_trace.close()
    if disconnect_observer:
        trace.expect_disconnect()
        request('disconnect')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--accept-eula', action='store_true', required=True)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--jars', type=Path, default=REPO / '.local/climbing/downloads')
    parser.add_argument('--version', choices=['1.16.1', '1.21.11'], action='append')
    parser.add_argument('--compiled-sdk-revision', required=True)
    args = parser.parse_args()
    revision = subprocess.check_output(['git', 'rev-parse', 'HEAD'], cwd=REPO, text=True).strip()
    compiled = subprocess.check_output(['git', 'rev-parse', args.compiled_sdk_revision], cwd=REPO, text=True).strip()
    paths = ['src', 'examples', 'Cargo.toml', 'Cargo.lock', 'data']
    assert not subprocess.check_output(['git', 'diff', compiled, 'HEAD', '--', *paths], cwd=REPO)
    assert not subprocess.check_output(['git', 'diff', 'HEAD', '--', *paths], cwd=REPO)
    sdk = dict(source_revision=revision, binary_build_revision=compiled,
               binary_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest())
    for version in args.version or ['1.16.1', '1.21.11']:
        run(version, args.binary.resolve(), args.jars.resolve(),
            check=functools.partial(check, binary=args.binary.resolve(), sdk=sdk))


if __name__ == '__main__':
    main()
