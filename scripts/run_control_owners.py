#!/usr/bin/env python3
"""Verify selected control IDs on isolated unchanged official servers."""
import argparse
from pathlib import Path
import re

from run_climbing_control import REPO, run
from run_common_native import PacketTraceProxy


def check(version, command, request, trace, report):
    command('tp ClimbingProbe 0.5 65 0.5')
    request('prepare', position=[.5,65.,.5])
    keys = dict(forward=0, strafe=0, jump=False, sneak=True, sprint=False, yaw=0., pitch=0.)
    first = request('start')
    selected = request('keys_for', session_id=first['session_id'], controls=keys)
    assert selected['admitted'] and selected['record']['session_id']==first['session_id']
    request('ticks', count=5)
    stopped = request('stop_for', session_id=first['session_id'])
    assert stopped['admitted'] and stopped['record']['status']['status']=='stopped'
    second = request('start')
    assert second['session_id'] != first['session_id']
    current = request('keys_for', session_id=second['session_id'], controls=keys)
    assert current['admitted']
    settled = request('ticks', count=5)

    def native_position():
        text = command('data get entity ClimbingProbe Pos')
        match = re.search(r'\[([^]]+)\]', text)
        assert match, text
        return [float(v.strip().rstrip('df')) for v in match[1].split(',')]

    before = native_position()
    boundary = trace.mark()
    stale_keys = request('keys_for', session_id=first['session_id'], controls=dict(keys,forward=1,sneak=False))
    stale_stop = request('stop_for', session_id=first['session_id'])
    for refusal in [stale_keys,stale_stop]:
        assert not refusal['admitted']
        assert refusal['record']['session_id']==second['session_id']
        assert refusal['record']['controls']==keys
        assert refusal['record']['status']['status']=='running'
    continued = request('ticks', count=5)
    assert continued['session_id']==second['session_id'] and continued['controls']==keys
    after = native_position()
    assert max(abs(a-b) for a,b in zip(before,after)) < .01, (before,after)
    frames = [f for f in trace.since(boundary) if f['phase']=='play' and f['direction']=='serverbound']
    # Existing controller ticks continue. A stale stop must not emit a release.
    action_id = 0x1c if version=='1.16.1' else 0x29
    actions = []
    for f in frames:
        if f['packet_id']==action_id:
            body=bytes.fromhex(f['body_hex'])
            _,offset=PacketTraceProxy.varint(body)
            action,_=PacketTraceProxy.varint(body[offset:])
            actions.append(action)
    releases = [1,4] if version=='1.16.1' else [2]
    assert not any(a in releases for a in actions), actions
    if version=='1.21.11':
        assert not any(f['packet_id']==0x2a and f['body_hex']=='00' for f in frames)
    report['checks'].append(dict(name='selected_inputs_and_stop_are_admitted', first=first,
        selected=selected, stopped=stopped, replacement=second, current=current, settled=settled))
    report['checks'].append(dict(name='stale_id_cannot_change_or_stop_replacement', stale_keys=stale_keys,
        stale_stop=stale_stop, continued=continued, native_before=before,native_after=after,original_frames=frames))
    stopped = request('stop_for', session_id=second['session_id'])
    assert stopped['admitted'] and stopped['record']['status']['status']=='stopped'
    later = request('wait', ms=200)
    assert later==stopped['record']
    repeated = request('stop_for', session_id=second['session_id'])
    assert repeated['admitted'] and repeated['record']==later
    report['checks'].append(dict(name='selected_stop_retains_record_without_automatic_restart',
        stopped=stopped, later=later, repeated=repeated))
    report['synthetic_fixture_not_historical_reproduction']=True
    report['limits']='Session IDs select within this connection; no capability, physical stop acknowledgement or global atomic capture is inferred.'
    trace.expect_disconnect()
    request('disconnect')


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--accept-eula',action='store_true',required=True)
    parser.add_argument('--binary',type=Path,default=REPO/'target/debug/examples/climbing_control_probe')
    parser.add_argument('--jars',type=Path)
    args=parser.parse_args()
    for version in ['1.16.1','1.21.11']:
        run(version,args.binary.resolve(),args.jars,check=check)
