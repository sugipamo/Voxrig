#!/usr/bin/env python3
"""Owned handles against an official 1.16.1 server, original-frame gates and RCON.

Proxy deferral is not an internal writer barrier. Header/body/flush races belong
in BoundedWriter tests. No original click or dig is fabricated or replayed here.
"""
import argparse
import hashlib
import sys
import threading
from pathlib import Path

sys.dont_write_bytecode = True
import run_climbing_control as harness
from run_common_native import PacketTraceProxy, until


class ReceiptGate(PacketTraceProxy):
    def __init__(self, *args, **kwargs):
        super().__init__(*args, **kwargs)
        self.gate = threading.Lock()
        self.receipts = False
        self.take = False
        self.held = []

    def forward_frame(self, record, destination, wire):
        with self.gate:
            play = record['phase'] == 'play'
            receipt = (play and self.receipts and record['direction'] == 'clientbound'
                       and record['packet_id'] in {0x12, 0x14, 0x16})
            take = (play and self.take and record['direction'] == 'serverbound'
                    and record['packet_id'] == 0x09
                    and int.from_bytes(bytes.fromhex(record['body_hex'])[1:3], 'big', signed=True) == 0)
            if receipt or take:
                self.held.append((record, destination, wire))
                return
            super().forward_frame(record, destination, wire)

    def release(self):
        with self.gate:
            for record, destination, wire in self.held:
                super().forward_frame(record, destination, wire)
            self.held.clear()
            self.receipts = self.take = False


def check(version, command, request, trace, report):
    assert version == '1.16.1'
    report['binary_sha256'] = hashlib.sha256(BINARY.read_bytes()).hexdigest()
    report['source_sha256'] = hashlib.sha256((Path(__file__).resolve().parents[1] / 'src/client/owned_operation.rs').read_bytes()).hexdigest()

    def ask(name, **kwargs):
        value = request(name, **kwargs)
        assert 'error' not in value, (name, kwargs, value)
        return value

    def item(value, name, count):
        return (value and value['value']['kind'] == 'item'
                and value['value']['item']['name'] == 'minecraft:' + name
                and value['value']['item']['count'] == count)

    def gate_ready():
        return until(lambda: r if (r := ask('inventory_send_record')) is None or r['status'] in ('received', 'diverged', 'not_sent') else None, 5)

    def click(slot):
        done = ask('click', slot=slot)
        assert done['outcome'] == 'SentReceived', done
        gate_ready()
        return done

    def prepare():
        before = ask('player')['receive_sequence']
        command('tp ClimbingProbe .5 65 .5 0 0')
        ask('prepare', position=[.5, 65., .5], after_sequence=before)

    def stock(name, count=1):
        before = ask('player')['receive_sequence']
        command('clear ClimbingProbe')
        command(f'replaceitem entity ClimbingProbe inventory.0 minecraft:{name} {count}')
        def fresh_stock():
            slot = ask('player')['inventory']['slots'][9]
            return item(slot, name, count) and slot['source']['sequence'] > before
        until(fresh_stock, 5)
        # RCON slot edits can still have other slot/cursor updates in flight.
        # Establish an ordered full-content boundary before retaining the baseline.
        sync = ask('resync')
        settled = gate_ready()
        assert settled['id'] == sync['id'] and settled['dispatched'], settled

    def frames(mark, direction, packet):
        return [r for r in trace.since(mark) if r['phase'] == 'play'
                and r['direction'] == direction and r['packet_id'] == packet]

    def pending_then_late(kind, slot=None):
        mark = trace.mark()
        trace.receipts = True
        try:
            begun = ask('begin', kind=kind, budget_ms=150, **({'slot': slot} if slot is not None else {}))
            assert begun['waiter_dropped']
            until(lambda: len(trace.held) > 0, 5)
            done = ask('wait', id=begun['id'])
            assert done['outcome'] == {'Unknown': 'Deadline'}, done
            assert not done['owner_joined']
            native = command('data get entity ClimbingProbe Inventory')
            original = frames(mark, 'serverbound', 0x09)
            assert len(original) == 1, original
            # A second invocation must not release the first response debt.
            refused = ask('begin', kind='send_click', slot=11)
            denied = ask('wait', id=refused['id'])
            assert denied['failure'] is not None, denied
            assert len(frames(mark, 'serverbound', 0x09)) == 1
            retained = ask('snapshot', id=begun['id'])
            assert retained['completion'] == done
        finally:
            trace.release()
        late = until(lambda: s['joined'] if (s := ask('snapshot', id=begun['id']))['joined'] is not None else None, 5)
        assert late['id'] == done['id']
        assert late['scope'] == done['scope']
        assert late['outcome'] == 'SentReceived', late
        assert ask('wait', id=begun['id']) == done
        gate_ready()
        assert len(frames(mark, 'serverbound', 0x09)) == 1
        report['checks'].append(dict(name=kind + '_waiter_drop_deadline_receipts_late', begin=begun,
                                     terminal=done, late=late, native=native, frames=trace.since(mark)))
        return late

    prepare()
    # Selected stop and replacement owner identity; no server ACK is invented.
    started = ask('start')
    mark = trace.mark()
    controls = dict(forward=1, strafe=0, jump=False, sneak=False, sprint=True, yaw=0., pitch=0.)
    ask('keys', controls=controls)
    until(lambda: [f for f in frames(mark, 'serverbound', 0x1c)
                   if bytes.fromhex(f['body_hex'])[-2] == 3], 5)
    begun = ask('begin', kind='stop', session_id=started['session_id'])
    stopped = ask('wait', id=begun['id'])
    assert stopped['outcome'] == 'SentUnconfirmed', stopped
    assert stopped['record']['session_id'] == started['session_id']
    assert stopped['record']['status']['status'] == 'stopped'
    release_frames = until(lambda: [f for f in frames(mark, 'serverbound', 0x1c)
                                   if bytes.fromhex(f['body_hex'])[-2] == 4], 5)
    noop = ask('begin', kind='stop', session_id=started['session_id'])
    assert ask('wait', id=noop['id'])['outcome'] == 'NotSent'
    successor = ask('start')
    old = ask('begin', kind='stop', session_id=started['session_id'])
    assert ask('wait', id=old['id'])['outcome'] == 'NotSent'
    assert ask('record')['session_id'] == successor['session_id']
    assert ask('record')['status']['status'] in ('running', 'paused')
    stop_next = ask('begin', kind='stop', session_id=successor['session_id'])
    ask('wait', id=stop_next['id'])
    assert ask('wait', id=begun['id']) == stopped
    report['checks'].append(dict(name='selected_stop_drop_noop_and_successor', stopped=stopped,
                                 successor=successor, release_frames=release_frames,
                                 native=command('data get entity ClimbingProbe Pos')))
    prepare()

    begun = ask('begin', kind='look')
    look = ask('wait', id=begun['id'])
    assert look['outcome'] == 'SentUnconfirmed', look
    native = until(lambda: r if '37.0f' in (r := command('data get entity ClimbingProbe Rotation')) and '13.0f' in r else None, 5)
    prepare()
    corrected = until(lambda: r if (r := ask('body_status', id=look['record']['id']))['status'] == 'corrected' else None, 5)
    report['checks'].append(dict(name='body_owned_send_and_late_corrected', look=look, native=native, corrected=corrected))
    begun = ask('begin', kind='move', controls=dict(controls, sprint=False), ticks=4)
    moved = ask('wait', id=begun['id'])
    assert moved['outcome'] == 'SentUnconfirmed', moved
    until(lambda: r if (r := ask('record'))['status']['status'] == 'stopped' and r['dispatched_ticks'] >= 4 else None, 5)
    until(lambda: ask('body_status', id=moved['record']['id'])['status'] != 'pending', 5)
    ask('body_release')
    report['checks'].append(dict(name='body_owned_initial_send_keeps_finite_ticks', dispatch=moved, record=ask('record')))
    prepare()

    stock('stone', 32)
    pending_then_late('click', 9)
    click(10)  # Preserve the actual cursor through a separate original click.
    stock('stone', 32)
    pending_then_late('transfer', 9)
    stock('stone', 32)
    pending_then_late('send_click', 9)
    click(10)
    stock('stone', 32)
    pending_then_late('send_transfer', 9)

    def grid():
        stock('oak_log')
        click(9)
        click(1)
        until(lambda: item(ask('crafting')['result'], 'oak_planks', 4), 5)

    grid()
    pending_then_late('take')
    click(10)
    grid()
    pending_then_late('shift_take')

    # Same original result click, independent actual collect, explicit continuation.
    grid()
    mark = trace.mark()
    trace.take = True
    try:
        begun = ask('begin', kind='operational')
        until(lambda: len(trace.held) == 1, 5)
        command('summon minecraft:item .5 65 .5 {Item:{id:"minecraft:oak_log",Count:1b},PickupDelay:0s}')
        until(lambda: frames(mark, 'clientbound', 0x55), 10)
    finally:
        trace.release()
    crafted = ask('wait', id=begun['id'])
    assert crafted['outcome'] == 'SentReceived', crafted
    record = crafted['record']
    assert record['operational']['stage'] == 'crafted', record
    assert len(record['operational']['external']) == 1, record
    assert record['strict']['stage'] == 'requires_inspection', record
    assert ask('continue')['released']
    strict_before = ask('take_record')
    next_send = ask('begin', kind='send_click', slot=10)
    placed = ask('wait', id=next_send['id'])
    assert placed['outcome'] == 'SentReceived', placed
    assert ask('take_record')['stage'] == strict_before['stage']
    report['checks'].append(dict(name='operational_drop_external_strict_inspection_explicit_continue',
                                 crafted=crafted, next_send=placed,
                                 native=command('data get entity ClimbingProbe Inventory'), frames=trace.since(mark)))
    trace.expect_disconnect()
    ask('disconnect')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--accept-eula', action='store_true', required=True)
    parser.add_argument('--binary', type=Path, required=True)
    parser.add_argument('--jars', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    BINARY = args.binary.resolve()
    harness.ROOT = args.output.resolve()
    harness.run('1.16.1', BINARY, args.jars.resolve(), check=check, trace_factory=ReceiptGate)
