#!/usr/bin/env python3
"""Mode-bound common ground control on unchanged official loopback servers."""
import argparse
import functools
import hashlib
import math
from pathlib import Path
import re
import struct
import subprocess
from run_climbing_control import REPO, run
from run_common_native import until


def check(version, command, request, trace, report, sdk):
    report['sdk']=sdk
    def save(name, **facts):report['checks'].append(dict(name=name,**facts))
    def native_position():
        text=command('data get entity ClimbingProbe Pos')
        match=re.search(r'\[([^]]+)\]',text)
        assert match,text
        return [float(v.strip().rstrip('df')) for v in match[1].split(',')]
    def received_mode(mode):
        return until(lambda:(p if (p:=request('player'))['game_mode']==mode else None),5)
    def controls(**changes):
        return dict(forward=changes.get('forward',0),strafe=changes.get('strafe',0),
                    jump=changes.get('jump',False),sneak=changes.get('sneak',False),
                    sprint=changes.get('sprint',False),yaw=37.,pitch=-12.)
    def op(action,mode,session=None,**kw):
        if session is not None:kw['session_id']=session
        return request('mode_'+action,mode=mode,**kw)
    def admitted(action,mode,session=None,**kw):
        result=op(action,mode,session,**kw)
        assert result['admitted'],result
        return result.get('record',result.get('request'))
    def rejected(action,mode,session=None,**kw):
        result=op(action,mode,session,**kw)
        assert not result['admitted'],result
        return result
    move_id=0x13 if version=='1.16.1' else 0x1e
    def moves(boundary):
        return [p for p in trace.since(boundary) if p['phase']=='play'
                and p['direction']=='serverbound' and p['packet_id']==move_id]
    def capture(name,record,boundary):
        assert record['status']['status']=='running' and not record['corrections'],record
        # A complete SDK write can precede the original server's queued packet
        # handling. Poll only the independent observation, never replay inputs.
        # Keep the same .4 coordinate bound; a missed observation window fails.
        def applied_position():
            actual=native_position()
            return actual if max(abs(a-b) for a,b in zip(actual,record['frame']['position']))<.4 else None
        actual=until(applied_position,2)
        frame=record['frame']
        expected=struct.pack('>dddff',*frame['position'],record['controls']['yaw'],record['controls']['pitch'])
        expected+=bytes([int(frame['on_ground']) | (int(frame['horizontal_collision'])<<1 if version=='1.21.11' else 0)])
        def written():
            wire=moves(boundary)
            return wire if any(p['body_hex']==expected.hex() for p in wire) else None
        wire=until(written,5)
        save(name,record=record,native_position=actual,movement_packets=wire,
             original_mode=command('data get entity ClimbingProbe playerGameType'),
             original_abilities=command('data get entity ClimbingProbe abilities'))
        return actual
    command('fill -12 64 -12 12 64 12 minecraft:stone')
    command('fill -8 65 -8 8 80 8 minecraft:air')
    for mode in ['adventure','creative']:
        command('gamemode '+mode+' ClimbingProbe');player=received_mode(mode)
        command('tp ClimbingProbe 0.5 65 0.5 37 -12');request('prepare',position=[.5,65.,.5])
        start=admitted('start',mode);session=start['session_id']
        assert start['mode']==mode and start['controls']==controls(),start
        refusals=[rejected('keys','survival',session,controls=controls(forward=1)),
                  rejected('jump','survival',session),rejected('stop','survival',session),
                  rejected('keys',mode,session+1,controls=controls(forward=1))]
        assert request('record')['controls']==controls()
        save(mode+'_other_handle_and_stale_id_refused',start=start,player=player,refusals=refusals)
        before=native_position()
        for axis,f,st in [('forward',1,0),('backward',-1,0),('left',0,1),('right',0,-1)]:
            boundary=trace.mark();admitted('keys',mode,session,controls=controls(forward=f,strafe=st))
            record=request('ticks',count=10);actual=capture(mode+'_'+axis,record,boundary)
            delta=[actual[i]-before[i] for i in range(3)]
            rad=math.radians(37.); direction=[st*math.cos(rad)-f*math.sin(rad),0,st*math.sin(rad)+f*math.cos(rad)]
            assert sum(delta[i]*direction[i] for i in [0,2])>.2,(axis,delta)
            before=actual
        admitted('keys',mode,session,controls=controls())
        request('ticks',count=12)
        boundary=trace.mark();admitted('keys',mode,session,controls=controls(jump=True))
        record=request('ticks',count=42);capture(mode+'_held_jump',record,boundary)
        heights=[struct.unpack('>ddd',bytes.fromhex(p['body_hex'])[:24])[1] for p in moves(boundary)]
        assert sum(b>a+1e-8 and (i==0 or a<=heights[i-1]+1e-8) for i,(a,b) in enumerate(zip(heights,heights[1:])))>=2,heights
        admitted('keys',mode,session,controls=controls());request('ticks',count=30)
        assert request('record')['frame']['on_ground']
        boundary=trace.mark();queued=admitted('jump',mode,session)
        record=request('ticks',count=32);capture(mode+'_one_shot_ground_jump',record,boundary)
        assert record['ground_jump']['status']['outcome']=='applied' and record['ground_jump']['status']['dispatched']
        heights=[65.]+[struct.unpack('>ddd',bytes.fromhex(p['body_hex'])[:24])[1] for p in moves(boundary)]
        assert sum(b>a+1e-8 and (i==0 or a<=heights[i-1]+1e-8) for i,(a,b) in enumerate(zip(heights,heights[1:])))==1,heights
        save(mode+'_queued_once',request=queued)
        boundary=trace.mark();admitted('keys',mode,session,controls=controls(forward=1,sprint=True))
        record=request('ticks',count=8);capture(mode+'_sprint',record,boundary);assert record['frame']['sprinting']
        boundary=trace.mark();admitted('keys',mode,session,controls=controls(sneak=True))
        record=request('ticks',count=5);capture(mode+'_crouch',record,boundary);assert record['frame']['crouching']
        boundary=trace.mark();stopped=admitted('stop',mode,session)
        assert stopped['status']['status']=='stopped'
        assert request('wait',ms=150)==stopped
        releases=[p for p in trace.since(boundary) if p['phase']=='play' and p['direction']=='serverbound']
        if version=='1.21.11':
            assert any(p['packet_id']==0x2a and p['body_hex']=='00' for p in releases),releases
        else:
            from run_common_native import PacketTraceProxy
            def unsneak(p):
                if p['packet_id']!=0x1c:return False
                body=bytes.fromhex(p['body_hex']);_,offset=PacketTraceProxy.varint(body)
                return PacketTraceProxy.varint(body[offset:])[0]==1
            assert any(unsneak(p) for p in releases),releases
        save(mode+'_owned_release_retained',record=stopped,release_packets=releases)
        next_start=admitted('start',mode);replacement=next_start['session_id']
        assert replacement!=session and next_start['controls']==controls()
        stale=rejected('stop',mode,session)
        save(mode+'_neutral_explicit_restart',record=next_start,stale=stale)
        admitted('keys',mode,replacement,controls=controls(forward=1,sneak=True));request('ticks',count=3)
        command('gamemode survival ClimbingProbe');received_mode('survival')
        stopped=until(lambda:(r if (r:=request('record'))['status']['status']=='stopped' else None),5)
        assert stopped['mode']==mode and stopped['mode_change_sequence'] is not None
        peers=[p for p in trace.since(0) if p['direction']=='clientbound' and p['phase'] in ('configuration','play')]
        original=peers[stopped['mode_change_sequence']-1]
        assert original['phase']=='play' and original['packet_id']==(0x1e if version=='1.16.1' else 0x26)
        assert bytes.fromhex(original['body_hex'])==b'\x03'+struct.pack('>f',0.)
        command('gamemode '+mode+' ClimbingProbe');received_mode(mode)
        assert request('wait',ms=120)==stopped
        save(mode+'_received_mode_change_fences_old_owner',record=stopped,original_mode_packet=original,
             keys_refusal=rejected('keys',mode,replacement,controls=controls(forward=1)),
             jump_refusal=rejected('jump',mode,replacement))
        fresh=admitted('start',mode);assert fresh['mode_change_sequence'] is None
        admitted('stop',mode,fresh['session_id'])
    command('gamemode spectator ClimbingProbe');received_mode('spectator')
    boundary=trace.mark();refusal=rejected('start','spectator')
    assert refusal['kind']=='Unsupported',refusal
    assert not moves(boundary)
    save('spectator_ground_model_refused_before_dispatch',refusal=refusal)
    trace.expect_disconnect();request('disconnect')


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--accept-eula',action='store_true',required=True)
    p.add_argument('--version',choices=('1.16.1','1.21.11'),action='append')
    p.add_argument('--binary',type=Path,required=True)
    p.add_argument('--jars',type=Path,default=REPO/'.local/climbing/downloads')
    p.add_argument('--compiled-sdk-revision',required=True)
    args=p.parse_args()
    revision=subprocess.check_output(['git','rev-parse','HEAD'],cwd=REPO,text=True).strip()
    compiled=subprocess.check_output(['git','rev-parse',args.compiled_sdk_revision],cwd=REPO,text=True).strip()
    if subprocess.check_output(['git','diff',compiled,'HEAD','--','src','examples','Cargo.toml','Cargo.lock','data'],cwd=REPO):raise RuntimeError('compiled SDK differs from runtime source')
    files=('Cargo.lock','scripts/run_ground_modes.py','scripts/run_climbing_control.py','scripts/run_common_native.py','examples/climbing_control_probe.rs','scripts/movement_oracle/run.py','scripts/movement_oracle/ground_mode_scenarios.json','scripts/movement_oracle/java_1_16_1/MovementOracle.java','scripts/movement_oracle/java_1_21_11/MovementOracle.java','data/client_api/ground_mode_oracle.json.gz')
    sdk=dict(source_revision=revision,binary_build_revision=compiled,binary_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest(),source_diff_sha256=hashlib.sha256(subprocess.check_output(['git','diff','HEAD'],cwd=REPO)).hexdigest(),artifacts={f:hashlib.sha256((REPO/f).read_bytes()).hexdigest() for f in files})
    for version in args.version or ('1.16.1','1.21.11'):
        run(version,args.binary.resolve(),args.jars.resolve(),check=functools.partial(check,sdk=sdk))
if __name__=='__main__':main()
