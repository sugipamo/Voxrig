#!/usr/bin/env python3
"""Body digging without an estimate on local SHA-1 checked official servers.

Original-byte proxy only; no SDK cache edits or fabricated ACKs. Independent
RCON verifies air, tools, water/ground and explicit fixture rollback updates.
"""
import argparse
import hashlib
import os
import shutil
import threading
import time
from pathlib import Path

import run_climbing_control as harness
from run_common_native import PacketTraceProxy, until


class InitialInventoryDelay(PacketTraceProxy):
    """Withhold original inventory frames while delivering world/pose/ACKs.

    No frame bytes are edited. Initial inventory frames are released in their
    original relative order after the unknown-hand test, then normal ordering
    resumes. This models missing knowledge without patching SDK state.
    """
    def __init__(self, *args):
        self.inventory_lock = threading.Lock()
        self.initial_inventory = []
        self.inventory_pending = True
        self.effect_removal_pending = False
        self.effect_removals = []
        super().__init__(*args)

    def forward_frame(self, record, destination, wire):
        self.before_forward(record)
        inventory = {0x14, 0x16} if self.version == '1.16.1' else {0x12, 0x14, 0x6a}
        with self.inventory_lock:
            if (self.inventory_pending and record['phase'] == 'play' and
                    record['direction'] == 'clientbound' and record['packet_id'] in inventory):
                self.initial_inventory.append((destination, bytes(wire)))
            elif (self.effect_removal_pending and record['phase'] == 'play' and
                  record['direction'] == 'clientbound' and record['packet_id'] == (0x38 if self.version == '1.16.1' else 0x4c)):
                self.effect_removals.append((destination, bytes(wire)))
            else:
                destination.sendall(wire)

    def release_inventory(self):
        with self.inventory_lock:
            count = len(self.initial_inventory)
            for destination, wire in self.initial_inventory:
                destination.sendall(wire)
            self.initial_inventory.clear()
            self.inventory_pending = False
            return count

    def release_effects(self):
        with self.inventory_lock:
            count = len(self.effect_removals)
            for destination, wire in self.effect_removals:
                destination.sendall(wire)
            self.effect_removals.clear()
            self.effect_removal_pending = False
            return count


def check(version, command, request, trace, report):
    legacy = version == '1.16.1'
    packet = 0x1b if legacy else 0x28
    target = [0, 65, 2]
    report['binary_sha256'] = hashlib.sha256(BINARY.read_bytes()).hexdigest()
    report['timeout_ms'] = 2000

    def observe():
        return request('observe', blocks=[target])

    def state(name):
        return until(lambda: (s if s and s['value']['name'] == 'minecraft:' + name else None)
                     if (s := observe()['blocks'][0]['state']) else None, 5)

    def setblock(name):
        command('setblock 0 65 2 minecraft:' + name)
        state(name)

    def frames(mark):
        return [f for f in trace.since(mark) if f['phase'] == 'play' and
                f['direction'] == 'serverbound' and f['packet_id'] == packet and
                bytes.fromhex(f['body_hex'])[0] in (0, 1, 2)]

    def complete(identity):
        assert 'error' not in identity, identity
        result = until(lambda: (s if s['status'] != 'pending' else None)
                       if (s := request('status', id=identity)) else None, 5)
        request('wait', ms=100)  # includes bounded owned cleanup
        return result

    def action(name, expect=('accepted',), **args):
        mark, started = trace.mark(), time.monotonic()
        identity = request('submit', kind='break', pos=args.get('pos', target), face='North')
        result = complete(identity)
        entry = dict(name=name, id=identity, result=result, seconds=time.monotonic()-started,
                     frames=frames(mark), diagnostic=request('dig_estimate', pos=args.get('pos', target)))
        entry['statuses'] = [bytes.fromhex(f['body_hex'])[0] for f in entry['frames']]
        report['checks'].append(entry)
        assert result['status'] in expect, entry
        assert entry['seconds'] < 3.5, entry
        if result['status'] == 'rejected':
            assert entry['statuses'] == [], entry
        else:
            assert entry['statuses'].count(0) == 1, entry
            assert entry['statuses'].count(2) <= 1 and entry['statuses'].count(1) <= 1, entry
        if result['status'] == 'accepted':
            native = command('execute if block 0 65 2 minecraft:air')
            assert 'Test passed' in native, native
            entry['native_air'] = native
        return entry

    def prepare(y=65.):
        command(f'tp ClimbingProbe .5 {y} .5 0 0')
        assert 'error' not in request('prepare_pose', position=[.5, y, .5])

    command('fill -3 64 -3 3 64 4 minecraft:stone')
    command('fill -3 65 -3 3 70 4 minecraft:air')
    prepare()
    assert 'error' not in request('prepare', position=[.5, 65., .5])
    setblock('dirt')
    # Exact original initial inventory frames are withheld while world/pose and
    # processing ACKs pass. The actual selected hand observation is unavailable.
    report['initial_player'] = request('player')
    estimate = request('dig_estimate', pos=target)
    assert 'error' in estimate, estimate
    mark = trace.mark()
    strict = request('strict_dig', pos=target)
    assert 'error' in strict and not frames(mark), strict
    report['checks'].append(dict(name='strict_unknown_estimate_unchanged', estimate=estimate, result=strict))
    unknown = action('unknown_initial_estimate_sent')
    unknown['initial_estimate'] = estimate
    report['released_original_inventory_frames'] = trace.release_inventory()
    assert report['released_original_inventory_frames'] > 0
    until(lambda: request('player')['inventory']['slots'][36] is not None, 5)

    identity = request('submit', kind='select', slot=0)
    assert complete(identity)['status'] == 'accepted'
    replace = ('replaceitem entity ClimbingProbe weapon.mainhand ' if legacy else
               'item replace entity ClimbingProbe weapon.mainhand with ')

    def tool(name):
        command(replace + 'minecraft:' + name)
        def received():
            slot = request('player')['inventory']['slots'][36]
            return slot if slot and isinstance(slot['value'], dict) and slot['value'].get('item', {}).get('name') == 'minecraft:' + name else None
        until(received, 5)

    for name in ['wooden_shovel', 'diamond_shovel', 'diamond_pickaxe']:
        tool(name)
        prepare()
        setblock('dirt')
        before = request('dig_estimate', pos=target)
        result = action('tool_' + name)
        result['before_estimate'] = before

    # Server on-ground and eye-water can disagree with a cached client estimate.
    # Body sends no movement prediction to turn those unknowns into prerequisites.
    tool('diamond_shovel')
    setblock('dirt')
    prepare(66.)
    report['airborne_native'] = command('data get entity ClimbingProbe OnGround')
    airborne = action('airborne', expect=('accepted', 'unknown'))
    assert airborne['statuses'][0] == 0
    prepare()
    command('fill -1 65 -1 1 67 1 minecraft:glass')
    command('fill 0 65 0 0 67 0 minecraft:water[level=0]')
    command('effect give ClimbingProbe minecraft:water_breathing 30 0 true')
    request('wait', ms=150)
    setblock('dirt')
    report['water_native'] = command('execute if block 0 66 0 minecraft:water')
    wet = action('in_water', expect=('accepted', 'unknown'))
    assert wet['statuses'][0] == 0
    command('fill -1 65 -1 1 67 1 minecraft:air')
    prepare()

    # Keep the actual fatigue-removal frame pending while native ACKs/blocks
    # pass. At submit the client estimator is provably too slow, while the
    # official server has already cleared fatigue and breaks at normal speed.
    command('effect give ClimbingProbe minecraft:mining_fatigue 30 2 true')
    request('wait', ms=150)
    setblock('dirt')
    trace.effect_removal_pending = True
    command('effect clear ClimbingProbe minecraft:mining_fatigue')
    until(lambda: len(trace.effect_removals), 3)
    stale = request('dig_estimate', pos=target)
    assert 'ticks' in stale and stale['ticks'] > 40, stale
    entry = action('wrong_live_timing_estimate_ignored')
    entry['estimate_at_submit'] = stale
    assert stale['ticks'] * .05 > entry['seconds'], entry
    report['released_real_effect_removal_frames'] = trace.release_effects()
    until(lambda: 'minecraft:mining_fatigue' not in request('player')['effects'], 3)

    # strict dig still waits for its estimator and returns its original record.
    setblock('dirt')
    mark = trace.mark()
    strict = request('strict_dig', pos=target)
    assert 'error' not in strict and strict['removed'], strict
    report['checks'].append(dict(name='strict_known_estimate_unchanged', result=strict, frames=frames(mark)))

    setblock('bedrock')
    action('known_unbreakable_kept', expect=('rejected',))
    action('official_far_reach_kept', expect=('rejected',), pos=[30, 65, 2])

    # Delay one real START processing reply and all later incoming bytes. The
    # deadline must send ABORT exactly once without guessing when STOP is legal.
    setblock('dirt')
    entered, release = threading.Event(), threading.Event()
    reply = 0x07 if legacy else 0x04
    def delay_start(frame):
        if frame['phase'] == 'play' and frame['direction'] == 'clientbound' and frame['packet_id'] == reply:
            entered.set()
            if not release.wait(5): raise RuntimeError('bounded START reply delay expired')
    trace.before_forward = delay_start
    try:
        timeout_result = action('start_ack_missing_deadline', expect=('unknown',))
        assert entered.is_set(), 'no real processing ACK intercepted'
        assert timeout_result['statuses'] == [0, 1], timeout_result
    finally:
        release.set()
        trace.before_forward = lambda frame: None
    request('wait', ms=200)

    setblock('dirt')
    command('effect give ClimbingProbe minecraft:mining_fatigue 30 2 true')
    request('wait', ms=150)
    slow = action('server_progress_exceeds_deadline', expect=('unknown',))
    assert slow['statuses'] == [0, 2, 1], slow
    command('effect clear ClimbingProbe minecraft:mining_fatigue')
    # ABORT does not retract a delayed destroy. Let the server's pending work
    # resolve before starting another explicit intent on the same target.
    until(lambda: 'Test passed' in command('execute if block 0 65 2 minecraft:air'), 5)
    request('wait', ms=100)
    assert request('status', id=slow['id'])['status'] == 'unknown'

    # Explicit actual server replacement during mining is a rollback notice.
    # The fixture update is distinguished from a spontaneous server rejection.
    setblock('dirt')
    command('effect give ClimbingProbe minecraft:mining_fatigue 30 2 true')
    request('wait', ms=150)
    mark = trace.mark()
    identity = request('submit', kind='break', pos=target, face='North')
    until(lambda: len(frames(mark)) >= 2, 2)
    command('setblock 0 65 2 minecraft:diamond_block')
    result = complete(identity)
    entry = dict(name='actual_server_replacement_corrected', result=result, frames=frames(mark))
    report['checks'].append(entry)
    assert result['status'] == 'corrected', entry
    assert [bytes.fromhex(f['body_hex'])[0] for f in entry['frames']] == [0, 2, 1], entry
    command('effect clear ClimbingProbe minecraft:mining_fatigue')
    trace.expect_disconnect()
    request('disconnect')


if __name__ == '__main__':
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--accept-eula', action='store_true', required=True)
    p.add_argument('--version', choices=['1.16.1', '1.21.11'], required=True)
    p.add_argument('--binary', type=Path, required=True)
    p.add_argument('--jars', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    args = p.parse_args()
    BINARY = args.binary.resolve()
    os.environ['VOXRIG_BODY_TIMEOUT_MS'] = '2000'
    args.output.mkdir(parents=True, exist_ok=True)
    harness.ROOT = args.output.resolve() / 'runs'
    try:
        harness.run(args.version, BINARY, args.jars.resolve(), check=check, trace_factory=InitialInventoryDelay)
    finally:
        if harness.ROOT.exists():
            for folder in harness.ROOT.iterdir():
                for name in ('report.json', 'packets.jsonl', 'probe.log', 'probe.stderr.log', 'server.log'):
                    if (folder / name).exists(): shutil.copyfile(folder / name, args.output / name)
