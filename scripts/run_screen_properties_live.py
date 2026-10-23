#!/usr/bin/env python3
"""Verify opening-bound property reads on a disposable official 1.21.11 server.

Anvil setup uses explicitly constructed test-only serverbound PICKUP inputs via
our proxy, not a supported common anvil operation. Clientbound frames stay
original. A receive barrier exposes the new OPEN before its initial properties.
"""
import argparse
import hashlib
import json
import re
import shutil
import struct
import tempfile
import threading
import zlib
from pathlib import Path

import run_climbing_control as harness
from run_common_native import PacketTraceProxy, until


def variable(value):
    result = bytearray()
    value &= 0xffffffff
    while True:
        byte = value & 127
        value >>= 7
        result.append(byte | (128 if value else 0))
        if not value:
            return bytes(result)


class SerializedSocket:
    def __init__(self, socket, lock):
        self.socket, self.lock = socket, lock

    def sendall(self, wire):
        with self.lock:
            self.socket.sendall(wire)

    def shutdown(self, how):
        self.socket.shutdown(how)


class FixtureProxy(PacketTraceProxy):
    def __init__(self, *args):
        self.send_lock = threading.Lock()
        self.release = threading.Event()
        self.release.set()
        self.reached = threading.Event()
        self.pause_property = False
        self.fixture_inputs = []
        super().__init__(*args)

    def forward(self, source, destination, direction, state):
        if direction == 'serverbound':
            destination = SerializedSocket(destination, self.send_lock)
        super().forward(source, destination, direction, state)

    def before_forward(self, record):
        if self.pause_property and record['direction'] == 'clientbound' and record['phase'] == 'play' and record['packet_id'] == 0x13:
            self.pause_property = False
            self.reached.set()
            if not self.release.wait(15):
                raise RuntimeError('property barrier timed out')

    def arm(self):
        self.reached.clear()
        self.release.clear()
        self.pause_property = True

    def fixture_pickup(self, window, revision, slot):
        # Original WINDOW_CLICK fields: container, revision, slot, button, PICKUP,
        # empty modified-hash map and a deliberately mismatching default-stone
        # cursor comparison, so the fixture receives an actual cursor resync.
        # This is test setup, never common click admission or success evidence.
        payload = variable(window) + variable(revision) + struct.pack('>h', slot) + bytes([0, 0, 0, 1, 1, 1, 0, 0])
        body = variable(0x11) + payload
        with self.send_lock:
            compression = self.connection_states[-1]['compression']
            if compression is not None:
                body = (variable(len(body)) + zlib.compress(body)) if len(body) >= compression else b'\0' + body
            wire = variable(len(body)) + body
            self.connections[-1][1].sendall(wire)
            self.fixture_inputs.append(dict(packet_id=0x11, payload_hex=payload.hex(), wire_sha256=hashlib.sha256(wire).hexdigest()))

    def close(self):
        self.release.set()
        super().close()


def check(version, command, request, trace, report):
    assert version == '1.21.11'
    command('tp ClimbingProbe 0.5 65 0.5 0 0')
    request('prepare', position=[0.5, 65.0, 0.5])
    request('select_hotbar', slot=0)
    command('tick freeze')

    def capture():
        return request('screen_properties')['screen']

    def source_sequence(value):
        assert value['source']['kind'] == 'received'
        return value['source']['sequence']

    def verify_original(screen):
        assert screen['id'] == request('screen')['screen']['id']
        window = screen['id']['window']
        incoming = [f for f in trace.frames if f['direction'] == 'clientbound' and f['phase'] not in ('handshake', 'login')]
        # State::receive advances once for every incoming protocol packet,
        # including configuration, after the separate login loop. Proxy ordinals include outgoing too.
        opening = incoming[screen['id']['opened_sequence'] - 1]
        assert opening['packet_id'] == 0x39 and opening['phase'] == 'play', opening
        for key, value in screen['values'].items():
            expected = variable(window) + struct.pack('>hh', int(key), value['value'])
            original = incoming[source_sequence(value) - 1]
            assert original['phase'] == 'play' and original['packet_id'] == 0x13 and original.get('body_hex') == expected.hex(), (key, value, original)
            assert source_sequence(value) > screen['id']['opened_sequence']
        assert not screen['evicted']
        return screen

    def open_menu(block, expected_keys, setup=None):
        previous = capture()
        if previous:
            request('close_screen')
        command('setblock 0 65 2 minecraft:air')
        command('setblock 0 65 2 minecraft:' + block)
        if setup:
            setup()
        trace.arm()
        try:
            request('use_on_block', support=[0, 65, 2], face='North', cursor=[0.5, 0.5, 0.0])
            assert trace.reached.wait(5), 'no initial property frame'
            empty = until(lambda: capture() if capture() and (not previous or capture()['id'] != previous['id']) else None, 5)
            assert empty['values'] == {} and not empty['evicted'], empty
        finally:
            trace.release.set()
        screen = until(lambda: capture() if capture() and set(capture()['values']) == set(expected_keys) else None, 5)
        verify_original(screen)
        return dict(before=previous, new_open_before_properties=empty, initial=screen)

    def furnace_setup():
        command('item replace block 0 65 2 container.0 with minecraft:raw_iron 1')
        command('item replace block 0 65 2 container.1 with minecraft:coal 2')

    furnace = open_menu('furnace', ['0', '1', '2', '3'], furnace_setup)
    command('tick step 20')
    cooked = until(lambda: capture() if capture()['values']['2']['value'] >= 20 and capture()['values']['0']['value'] > 0 else None, 6)
    command('tick step 20')
    advanced = until(lambda: capture() if capture()['values']['2']['value'] >= cooked['values']['2']['value'] + 20 else None, 6)
    native = command('data get block 0 65 2')
    for key, field in [('0', 'lit_time_remaining'), ('1', 'lit_total_time'), ('2', 'cooking_time_spent'), ('3', 'cooking_total_time')]:
        assert int(re.search(r'\b' + field + r': (-?\d+)s', native).group(1)) == advanced['values'][key]['value'], native
    furnace.update(progress=verify_original(cooked), advanced=verify_original(advanced), native=native)
    report['checks'].append(dict(name='furnace_burn_and_cook', **furnace))
    reset = open_menu('furnace', ['0', '1', '2', '3'])
    assert all(v['value'] == 0 for v in reset['initial']['values'].values())
    report['checks'].append(dict(name='furnace_reopen_clears_progress', **reset))

    command('item replace entity ClimbingProbe inventory.0 with minecraft:iron_sword[minecraft:damage=100] 1')
    command('item replace entity ClimbingProbe inventory.1 with minecraft:iron_ingot 1')
    anvil = open_menu('anvil', ['0'])
    def pickup(slot, expected_cursor_kind):
        screen = request('screen')['screen']
        before = screen['revision']['source']['sequence']
        trace.fixture_pickup(screen['id']['window'], screen['revision']['value'], slot)
        until(lambda: request('screen')['cursor'] and request('screen')['cursor']['value']['kind'] == expected_cursor_kind and request('screen')['cursor']['source']['sequence'] > before, 5)
    pickup(3, 'item')
    pickup(0, 'empty')
    pickup(4, 'item')
    pickup(1, 'empty')
    cost = until(lambda: capture() if capture()['values']['0']['value'] > 0 else None, 5)
    anvil.update(repair_cost=verify_original(cost), fixture_inputs=list(trace.fixture_inputs))
    report['checks'].append(dict(name='anvil_repair_cost', **anvil))

    def brewing_setup():
        command('item replace block 0 65 2 container.0 with minecraft:potion[minecraft:potion_contents={potion:"minecraft:water"}] 1')
        command('item replace block 0 65 2 container.3 with minecraft:nether_wart 1')
        command('item replace block 0 65 2 container.4 with minecraft:blaze_powder 1')
    brewing = open_menu('brewing_stand', ['0', '1'], brewing_setup)
    command('tick step 20')
    brewing_progress = until(lambda: capture() if 0 < capture()['values']['0']['value'] <= 381 else None, 6)
    command('tick step 20')
    brewing_advanced = until(lambda: capture() if 0 < capture()['values']['0']['value'] <= brewing_progress['values']['0']['value'] - 20 else None, 6)
    native = command('data get block 0 65 2')
    for key, field in [('0', 'BrewTime'), ('1', 'Fuel')]:
        assert int(re.search(r'\b' + field + r': (-?\d+)[sb]', native).group(1)) == brewing_advanced['values'][key]['value'], native
    brewing.update(progress=verify_original(brewing_progress), advanced=verify_original(brewing_advanced), native=native)
    report['checks'].append(dict(name='brewing_ticks_and_fuel', **brewing))
    reset = open_menu('brewing_stand', ['0', '1'])
    assert all(v['value'] == 0 for v in reset['initial']['values'].values())
    report['checks'].append(dict(name='brewing_reopen_clears_progress', **reset))
    request('close_screen')
    command('tick unfreeze')
    trace.expect_disconnect()
    request('disconnect')


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--accept-eula', action='store_true', required=True)
    parser.add_argument('--server-jar', type=Path, required=True)
    parser.add_argument('--binary', type=Path, default=harness.REPO / 'target/debug/examples/climbing_control_probe')
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    original_free_port = harness.free_port
    def free_port():
        port = original_free_port()
        while port in (25572, 25581):
            port = original_free_port()
        return port
    harness.free_port = free_port
    with tempfile.TemporaryDirectory(prefix='voxrig-screen-properties-') as temporary:
        root = Path(temporary)
        jars = root / 'jars'
        jars.mkdir()
        shutil.copyfile(args.server_jar, jars / '1.21.11-server.jar')
        harness.ROOT = root / 'runs'
        try:
            harness.run('1.21.11', args.binary.resolve(), jars, check=check, trace_factory=FixtureProxy)
        finally:
            if harness.ROOT.exists():
                for folder in harness.ROOT.iterdir():
                    for name in ('report.json', 'packets.jsonl', 'probe.log', 'probe.stderr.log', 'server.log'):
                        source = folder / name
                        if source.exists():
                            shutil.copyfile(source, args.output / name)
    sources = {str(p.relative_to(harness.REPO)): hashlib.sha256(p.read_bytes()).hexdigest()
               for base in ('src', 'data') for p in sorted((harness.REPO / base).rglob('*')) if p.is_file()}
    for name in ('examples/climbing_control_probe.rs', 'scripts/run_screen_properties_live.py', 'scripts/ExportScreenProperties.java'):
        sources[name] = hashlib.sha256((harness.REPO / name).read_bytes()).hexdigest()
    (args.output / 'inputs.json').write_text(json.dumps(dict(sources=sources, binary_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest()), indent=2) + '\n')


if __name__ == '__main__':
    main()
