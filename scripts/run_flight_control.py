#!/usr/bin/env python3
"""Held Creative/Spectator flight on unchanged official loopback servers."""
import argparse
import functools
import hashlib
import math
from pathlib import Path
import re
import struct
import subprocess
from run_climbing_control import REPO, run
from run_common_native import until, PacketTraceProxy


def check(version, command, request, trace, report, sdk):
    report['sdk']=sdk
    def save(name,**facts): report['checks'].append(dict(name=name,**facts))
    def native_position():
        text=command('data get entity ClimbingProbe Pos');match=re.search(r'\[([^]]+)\]',text)
        assert match,text
        return [float(v.strip().rstrip('df')) for v in match[1].split(',')]
    def received_mode(mode): return until(lambda:(p if (p:=request('player'))['game_mode']==mode else None),5)
    def controls(**kw):
        return dict(forward=kw.get('forward',0),strafe=kw.get('strafe',0),jump=kw.get('jump',False),sneak=kw.get('sneak',False),sprint=kw.get('sprint',False),yaw=37.,pitch=-12.)
    def op(action,mode,session=None,**kw):
        if session is not None:kw['session_id']=session
        return request(action,mode=mode,**kw)
    def admitted(action,mode,session=None,**kw):
        result=op(action,mode,session,**kw);assert result['admitted'],result
        return result['record']
    def rejected(action,mode,session=None,**kw):
        result=op(action,mode,session,**kw);assert not result['admitted'],result
        return result
    move_id=0x13 if version=='1.16.1' else 0x1e
    ability_id=0x1a if version=='1.16.1' else 0x27
    def outbound(boundary): return [p for p in trace.since(boundary) if p['phase']=='play' and p['direction']=='serverbound']
    def capture(name,record,boundary):
        assert record['kind']=='flight' and record['frame'] is not None and not record['corrections'],record
        frame=record['frame']
        def applied_position():
            pos=native_position()
            return pos if max(abs(a-b) for a,b in zip(pos,frame['position']))<.4 else None
        actual=until(applied_position,2)
        expected=struct.pack('>dddff',*frame['position'],record['controls']['yaw'],record['controls']['pitch'])
        expected+=bytes([int(frame['on_ground']) | ((int(frame['horizontal_collision'])<<1) if version=='1.21.11' else 0)])
        def written():
            packets=outbound(boundary)
            return packets if any(p['packet_id']==move_id and p['body_hex']==expected.hex() for p in packets) else None
        wire=until(written,5)
        params=record['flight_parameters'];assert params['spectator']==(record['mode']=='spectator')
        received=[p for p in trace.since(0) if p['direction']=='clientbound' and p['phase'] in ('configuration','play')]
        original=received[params['abilities_sequence']-1]
        assert original['phase']=='play' and original['packet_id']==(0x31 if version=='1.16.1' else 0x3e),original
        body=bytes.fromhex(original['body_hex']);assert len(body)==9 and body[0]&4
        assert struct.pack('>f',params['ability_flying_speed'])==body[1:5]
        save(name,record=record,native_position=actual,movement_packets=wire,original_abilities=command('data get entity ClimbingProbe abilities'),original_abilities_packet=original)
        return actual
    command('fill -12 64 -12 12 64 12 minecraft:stone')
    command('fill -10 65 -10 10 83 10 minecraft:air')
    for mode in ['creative','spectator']:
        command('gamemode '+mode+' ClimbingProbe');received_mode(mode)
        command('tp ClimbingProbe 0.5 72 0.5 37 -12');request('prepare',position=[.5,72.,.5])
        if mode=='creative':
            rejection=rejected('flight_start',mode,controls=controls(jump=True))
            boundary=trace.mark();enable=admitted('flight_enable',mode,flying=True)
            def enabled():
                packets=outbound(boundary)
                return packets if any(x['packet_id']==ability_id and x['body_hex']=='02' for x in packets) else None
            packets=until(enabled,5)
            save('creative_explicit_enable_required',refusal=rejection,enable=enable,packets=packets)
        start=admitted('flight_start',mode,controls=controls(jump=True));session=start['session_id']
        assert start['kind']=='flight' and start['mode']==mode
        refusals=[rejected('mode_keys','survival',session,controls=controls(forward=1)),rejected('mode_stop','survival',session),rejected('mode_jump',mode,session),rejected('mode_keys',mode,session+1,controls=controls(forward=1))]
        save(mode+'_mode_id_ground_jump_fences',start=start,refusals=refusals)
        boundary=trace.mark();record=request('ticks',count=6);capture(mode+'_takeoff',record,boundary);assert record['frame']['flying']
        admitted('mode_keys',mode,session,controls=controls());request('ticks',count=10)
        for axis,f,st in [('forward',1,0),('backward',-1,0),('left',0,1),('right',0,-1)]:
            before=native_position();boundary=trace.mark()
            admitted('mode_keys',mode,session,controls=controls(forward=f,strafe=st))
            record=request('ticks',count=10);after=capture(mode+'_'+axis,record,boundary)
            rad=math.radians(37);direction=[st*math.cos(rad)-f*math.sin(rad),st*math.sin(rad)+f*math.cos(rad)]
            assert sum((after[i]-before[i])*d for i,d in zip([0,2],direction))>.2,(before,after)
            admitted('mode_keys',mode,session,controls=controls());request('ticks',count=10)
        for name,key,sign in [('ascend','jump',1),('descend','sneak',-1)]:
            before=native_position();boundary=trace.mark();admitted('mode_keys',mode,session,controls=controls(**{key:True}))
            record=request('ticks',count=8);after=capture(mode+'_'+name,record,boundary);assert sign*(after[1]-before[1])>.2
            assert not record['frame']['crouching'] and not record['frame']['swimming']
            admitted('mode_keys',mode,session,controls=controls());request('ticks',count=10)
        boundary=trace.mark();admitted('mode_keys',mode,session,controls=controls(forward=1,sprint=True))
        record=request('ticks',count=8);capture(mode+'_sprint',record,boundary);assert record['frame']['sprinting']
        # Original flight retains horizontal inertia (0.91 damping). Release
        # is not rest: wait for native decay before asserting a hover.
        admitted('mode_keys',mode,session,controls=controls());request('ticks',count=60)
        boundary=trace.mark();record=request('ticks',count=8);capture(mode+'_neutral_hover',record,boundary)
        assert max(abs(v) for v in record['frame']['velocity'])<.005
        boundary=trace.mark();admitted('mode_keys',mode,session,controls=controls(sneak=True));request('ticks',count=2)
        stopped=admitted('mode_stop',mode,session);assert stopped['status']['status']=='stopped'
        assert request('wait',ms=150)==stopped
        released=outbound(boundary)
        if version=='1.21.11':assert any(p['packet_id']==0x2a and p['body_hex']=='00' for p in released)
        else:
            def unsneak(p):
                if p['packet_id']!=0x1c:return False
                body=bytes.fromhex(p['body_hex']);_,offset=PacketTraceProxy.varint(body)
                return PacketTraceProxy.varint(body[offset:])[0]==1
            assert any(unsneak(p) for p in released),released
        assert not any(p['packet_id']==ability_id and p['body_hex']=='00' for p in released),released
        save(mode+'_release_retained_without_disabling_flight',record=stopped,packets=released)
        fresh=admitted('flight_start',mode,controls=controls());assert fresh['session_id']!=session
        save(mode+'_explicit_neutral_restart',record=fresh,stale=rejected('mode_stop',mode,session))
        admitted('mode_keys',mode,fresh['session_id'],controls=controls(forward=1));request('ticks',count=2)
        command('gamemode survival ClimbingProbe');received_mode('survival')
        ended=until(lambda:(r if (r:=request('record'))['status']['status']=='stopped' else None),5)
        assert ended['mode']==mode and (ended['mode_change_sequence'] is not None or ended['flight_change_sequence'] is not None)
        command('gamemode '+mode+' ClimbingProbe');received_mode(mode)
        assert request('wait',ms=120)==ended
        save(mode+'_received_revocation_ends_owner',record=ended,refusal=rejected('mode_keys',mode,fresh['session_id'],controls=controls(forward=1)))
    for mode in ['creative','spectator']:
        command('gamemode '+mode+' ClimbingProbe');received_mode(mode)
        command('fill -3 70 3 3 78 3 minecraft:stone')
        command('tp ClimbingProbe 0.5 72 0.5 0 0');request('prepare',position=[.5,72.,.5])
        if mode=='creative':admitted('flight_enable',mode,flying=True)
        keys=controls(forward=1);keys['yaw']=0.;keys['pitch']=0.
        boundary=trace.mark();owner=admitted('flight_start',mode,controls=keys)
        record=request('ticks',count=22);actual=capture(mode+'_wall_collision_scope',record,boundary)
        if mode=='creative':assert actual[2]<2.71 and record['frame']['horizontal_collision'],record
        else:assert actual[2]>3.5 and not record['frame']['horizontal_collision'],record
        admitted('mode_stop',mode,owner['session_id'])
        command('fill -3 70 3 3 78 3 minecraft:air')
        command('fill -3 70 -3 3 78 3 minecraft:water[level=0]')
        command('tp ClimbingProbe 0.5 73 0.5 37 -12');request('prepare',position=[.5,73.,.5])
        boundary=trace.mark();owner=admitted('flight_start',mode,controls=controls(jump=True,forward=1))
        record=request('ticks',count=8);capture(mode+'_submerged_flight',record,boundary)
        assert record['frame']['in_water'] and not record['frame']['swimming'] and not record['frame']['crouching']
        admitted('mode_stop',mode,owner['session_id'])
        command('fill -3 70 -3 3 78 3 minecraft:air')
    # Descending into original floor must disable flight before the landing move,
    # retain the stopped model and allow explicit ground continuation.
    command('gamemode creative ClimbingProbe');received_mode('creative')
    command('tp ClimbingProbe 0.5 65.4 0.5 37 -12');request('prepare',position=[.5,65.4,.5])
    admitted('flight_enable','creative',flying=True)
    boundary=trace.mark();start=admitted('flight_start','creative',controls=controls(sneak=True))
    landed=until(lambda:(r if (r:=request('record'))['status']['status']=='stopped' else None),5)
    assert landed['status']['reason']=='creative flight model landed' and not landed['frame']['flying'],landed
    capture('creative_model_landing',landed,boundary)
    packets=outbound(boundary);disable=next(i for i,p in enumerate(packets) if p['packet_id']==ability_id and p['body_hex']=='00')
    frame=landed['frame'];payload=struct.pack('>dddff',*frame['position'],landed['controls']['yaw'],landed['controls']['pitch'])
    payload+=bytes([int(frame['on_ground']) | ((int(frame['horizontal_collision'])<<1) if version=='1.21.11' else 0)])
    assert any(p['packet_id']==move_id and p['body_hex']==payload.hex() for p in packets[disable+1:]),packets
    assert landed['flight_disable']['dispatched']
    save('creative_landing_requires_explicit_reenable',refusal=rejected('flight_start','creative',controls=controls(jump=True)))
    assert 'flying: 0b' in command('data get entity ClimbingProbe abilities')
    ground=admitted('mode_start','creative');assert ground['kind']=='ground'
    admitted('mode_stop','creative',ground['session_id']);save('creative_explicit_ground_after_landing',record=ground,packets=packets)
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
    if subprocess.check_output(['git','diff','HEAD','--','src','examples','Cargo.toml','Cargo.lock','data'],cwd=REPO):raise RuntimeError('uncommitted runtime SDK source')
    files=('Cargo.lock','scripts/run_flight_control.py','scripts/run_climbing_control.py','scripts/run_common_native.py','examples/climbing_control_probe.rs','scripts/movement_oracle/run.py','scripts/movement_oracle/flight_scenarios.json','scripts/movement_oracle/java_1_16_1/MovementOracle.java','scripts/movement_oracle/java_1_21_11/MovementOracle.java','data/client_api/flight_control_oracle.json.gz')
    sdk=dict(source_revision=revision,binary_build_revision=compiled,binary_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest(),source_diff_sha256=hashlib.sha256(subprocess.check_output(['git','diff','HEAD'],cwd=REPO)).hexdigest(),artifacts={f:hashlib.sha256((REPO/f).read_bytes()).hexdigest() for f in files})
    for version in args.version or ('1.16.1','1.21.11'):
        run(version,args.binary.resolve(),args.jars.resolve(),check=functools.partial(check,sdk=sdk))
if __name__=='__main__':main()
