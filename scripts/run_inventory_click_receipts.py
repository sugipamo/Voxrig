#!/usr/bin/env python3
"""Qualify unrelated received hand changes during one original native PICKUP."""
import argparse
import threading
import time
from pathlib import Path

from run_climbing_control import REPO, run
from run_common_native import PacketTraceProxy, until


class DelayedClickTrace(PacketTraceProxy):
    """Delay one original frame, preserving bytes and all serverbound ordering."""
    def __init__(self, *args):
        self.armed = False
        self.held = threading.Event()
        self.release = threading.Event()
        self.delayed_frame = None
        super().__init__(*args)

    def before_forward(self, record):
        if (self.armed and record['direction'] == 'serverbound'
                and record['phase'] == 'play'
                and record['packet_id'] == (0x09 if self.version == '1.16.1' else 0x11)):
            self.armed = False
            self.delayed_frame = dict(record)
            self.held.set()
            if not self.release.wait(10):
                raise TimeoutError('original click timing gate exceeded ten seconds')

    def close(self):
        self.release.set()
        super().close()


def check(version, command, request, trace, report):
    report['synthetic_fixture_not_historical_reproduction'] = True
    report['timing_fixture'] = 'One original serverbound frame delayed; no rewriting, injection or replay.'
    command('clear ClimbingProbe')
    command('replaceitem entity ClimbingProbe inventory.0 minecraft:stone 7'
            if version == '1.16.1' else
            'item replace entity ClimbingProbe inventory.0 with minecraft:stone 7')

    def received_source():
        player = request('player')
        slot = player['inventory']['slots'][9]
        return player if (slot and slot['value']['kind'] == 'item'
                          and slot['value']['item']['name'] == 'minecraft:stone'
                          and slot['value']['item']['count'] == 7
                          and player['inventory']['cursor']['value']['kind'] == 'empty'
                          and player['inventory']['slots'][45]['value']['kind'] == 'empty') else None

    before = until(received_source, 5)
    boundary = trace.mark()
    trace.armed = True
    try:
        submitted = request('inventory_click', slot=9, button='right', submit_only=True)
        assert trace.held.wait(2), 'original PICKUP was not held'
        assert submitted['stage'] == 'pending' and submitted['send']['dispatched']
        command('replaceitem entity ClimbingProbe weapon.offhand minecraft:oak_log 1'
                if version == '1.16.1' else
                'item replace entity ClimbingProbe weapon.offhand with minecraft:oak_log 1')

        def received_other_hand():
            player = request('player')
            slot = player['inventory']['slots'][45]
            return player if (slot and slot['value']['kind'] == 'item'
                              and slot['value']['item']['name'] == 'minecraft:oak_log'
                              and slot['source']['sequence'] > submitted['send']['after_sequence']) else None

        unrelated = until(received_other_hand, 5)
        pending = request('inventory_record')
        assert pending['id'] == submitted['id'] and pending['stage'] == 'pending'
        assert pending['source_receipt'] is None and pending['cursor_receipt'] is None
        assert unrelated['inventory']['slots'][9]['value']['item']['count'] == 7
        native_pending = command('data get entity ClimbingProbe Inventory')
    finally:
        trace.release.set()

    def observed():
        record = request('inventory_record')
        assert record['id'] == submitted['id'], 'original click was replaced'
        assert record['stage'] != 'requires_inspection', record
        return record if record['stage'] == 'observed_clicked' else None

    complete = until(observed, 5)
    assert complete['source_receipt']['value']['item']['count'] == 3
    assert complete['cursor_receipt']['value']['item']['count'] == 4
    if version == '1.16.1':
        assert complete['legacy_reply'] is not None
    after = request('player')
    assert after['inventory']['slots'][45]['value']['item']['name'] == 'minecraft:oak_log'
    native_after = command('data get entity ClimbingProbe Inventory')
    original = [f for f in trace.since(boundary) if f['phase'] == 'play'
                and f['direction'] == 'serverbound'
                and f['packet_id'] == (0x09 if version == '1.16.1' else 0x11)]
    assert len(original) == 1 and original[0]['wire_sha256'] == trace.delayed_frame['wire_sha256']
    report['checks'].append(dict(name='unrelated_received_offhand_update_during_original_pickup',
        before=before, submitted=submitted, unrelated=unrelated, pending=pending,
        complete=complete, after=after, native_pending=native_pending,
        native_after=native_after, original_click=original[0]))
    trace.expect_disconnect()
    request('disconnect')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--accept-eula', action='store_true', required=True)
    parser.add_argument('--binary', type=Path, default=REPO/'target/debug/examples/climbing_control_probe')
    parser.add_argument('--jars', type=Path)
    parser.add_argument('--version', choices=['1.16.1', '1.21.11'])
    args = parser.parse_args()
    for version in ([args.version] if args.version else ['1.16.1', '1.21.11']):
        run(version, args.binary.resolve(), args.jars, check=check, trace_factory=DelayedClickTrace)
