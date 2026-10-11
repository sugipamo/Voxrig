#!/usr/bin/env python3
"""Qualify owned player dimensions on unchanged official servers."""
import argparse
import functools
import hashlib
import json
import re
import struct
import subprocess
from pathlib import Path
from run_climbing_control import REPO, run
from run_common_native import PacketTraceProxy, until


def check(version,command,request,trace,report,sdk):
    report['sdk']=sdk;modern=version=='1.21.11';packet=0x1e if modern else 0x13
    def save(name,**facts):report['checks'].append(dict(name=name,**facts))
    def keys(**values):return dict(dict(forward=0,strafe=0,jump=False,sneak=False,sprint=False,yaw=0.,pitch=0.),**values)
    def admitted(result):assert result['admitted'],result;return result['record']
    def original(sequence):
        return until(lambda:frames[sequence-1] if len(frames:=[f for f in trace.since(0) if f['direction']=='clientbound' and f['phase'] in ('configuration','play')])>=sequence else None,5)
    def mode(value):
        command('gamemode '+value+' ClimbingProbe')
        until(lambda:request('player')['game_mode']==value,5)
    def scale(value):
        if not modern:return None
        boundary=request('player')['receive_sequence']
        command(f'attribute ClimbingProbe minecraft:scale base set {value}')
        reading=until(lambda:r if (r:=request('player')['attributes'].get('minecraft:scale')) and r['value']['value']==value and r['source']['kind']=='received' and r['source']['sequence']>boundary else None,5)
        wire=original(reading['source']['sequence']);assert wire['packet_id']==0x81
        body=bytes.fromhex(wire['body_hex']);entity,offset=PacketTraceProxy.varint(body)
        assert entity==request('player')['entity_id']
        assert body[offset:]==bytes([1,25])+struct.pack('>d',value)+b'\x00',wire
        return dict(reading=reading,original_packet=wire)
    def reset(position):
        command('tp ClimbingProbe '+' '.join(map(str,position))+' 0 0');request('prepare',position=position)
    def f32(value):return struct.unpack('>f',struct.pack('>f',value))[0]
    def inspect(name,record,value,boundary,receipt=None):
        assert record['status']['status']=='running' and not record['corrections'],record
        frame=record['frame'];s=f32(value);assert f32(frame['model_scale'])==s,record
        heights={'Standing':(1.8,1.62),'Crouching':(1.5,1.27),'Swimming':(.6,.4)}
        height,eye=heights[frame['pose']];width=f32(f32(.6)*s);height=f32(f32(height)*s);eye=f32(f32(eye)*s)
        assert abs((frame['bounds'][3]-frame['bounds'][0])-width)<1e-9,frame
        assert abs((frame['bounds'][4]-frame['bounds'][1])-height)<1e-9,frame
        assert abs((frame['eye_position'][1]-frame['position'][1])-eye)<1e-9,frame
        count=record['dispatched_ticks']
        def moves():return [f for f in trace.since(boundary) if f['direction']=='serverbound' and f['phase']=='play' and f['packet_id']==packet]
        until(lambda:len(moves())>=count,5)
        wire=moves()[count-1]
        flags=int(frame['on_ground']) | (int(frame['horizontal_collision'])<<1 if modern else 0)
        expected=struct.pack('>dddff',*frame['position'],record['controls']['yaw'],record['controls']['pitch'])+bytes([flags])
        assert wire['body_hex']==expected.hex(),(wire,record)
        text=command('data get entity ClimbingProbe Pos');match=re.search(r'\[([^]]+)\]',text);assert match,text
        native=[float(x.strip().removesuffix('d')) for x in match[1].split(',')]
        assert max(abs(a-b) for a,b in zip(native,frame['position']))<.6,(native,record)
        save(name,record=record,original_movement=wire,native_position=native,scale_receipt=receipt)
        return frame
    def stop(mode_,owner):
        stopped=admitted(request('mode_stop',mode=mode_,session_id=owner['session_id']))
        assert request('wait',ms=100)==stopped,stopped
        return stopped
    for value in ([.125,.25,.5,1.,2.,4.] if modern else [1.]):
        mode('survival');receipt=scale(value)
        command('fill -4 65 -4 4 84 4 minecraft:air')
        command('fill -4 65 3 4 74 3 minecraft:stone')
        reset([.5,65.,.5]);boundary=trace.mark();owner=admitted(request('mode_start',mode='survival'))
        admitted(request('mode_keys',mode='survival',session_id=owner['session_id'],controls=keys(forward=1)))
        walking=request('ticks',count=18);frame=inspect(f'scale_{value}_wall',walking,value,boundary,receipt)
        limit=3.-f32(f32(.6)*f32(value))/2.
        assert abs(frame['position'][2]-limit)<1e-8 and frame['horizontal_collision'],frame
        admitted(request('mode_keys',mode='survival',session_id=owner['session_id'],controls=keys(sneak=True)))
        crouch=request('ticks',count=3);inspect(f'scale_{value}_crouch',crouch,value,boundary,receipt)
        assert crouch['frame']['pose']=='Crouching',crouch
        save(f'scale_{value}_owned_stop',record=stop('survival',owner))
    for mode_,value in ([('creative',.25),('creative',2.),('creative',4.),('spectator',2.)] if modern else [('creative',1.)]):
        mode(mode_);receipt=scale(value)
        command('fill -4 65 -4 4 84 4 minecraft:air');reset([.5,74.,.5])
        if mode_=='creative':admitted(request('flight_enable',mode=mode_,flying=True))
        boundary=trace.mark();owner=admitted(request('flight_start',mode=mode_,controls=keys(forward=1,jump=True)))
        flying=request('ticks',count=4);inspect(f'{mode_}_{value}_scaled_flight',flying,value,boundary,receipt)
        assert flying['frame']['flying'] and flying['frame']['position'][1]>74.,flying
        stop(mode_,owner)
        if mode_=='creative':admitted(request('flight_enable',mode=mode_,flying=False))
    if modern:
        mode('survival');scale(1.);reset([.5,65.,.5])
        boundary=request('player')['receive_sequence']
        command('attribute ClimbingProbe minecraft:scale modifier add voxrig:scale_qualification .5 add_value')
        reading=until(lambda:r if (r:=request('player')['attributes'].get('minecraft:scale')) and r['value']['value']==1.5 and r['source']['sequence']>boundary else None,5)
        assert reading['value']['modifiers'],reading
        mark=trace.mark();owner=admitted(request('mode_start',mode='survival'));frame=inspect('actual_scale_modifier',request('ticks',count=3),1.5,mark,dict(reading=reading,original_packet=original(reading['source']['sequence'])))
        stop('survival',owner);command('attribute ClimbingProbe minecraft:scale modifier remove voxrig:scale_qualification')
        until(lambda:request('player')['attributes']['minecraft:scale']['value']['value']==1.,5)
        scale(16.);mark=trace.mark();refused=request('mode_start',mode='survival');assert not refused['admitted'] and 'unmodeled player scale' in refused['error'],refused
        request('wait',ms=100);assert not any(f['direction']=='serverbound' and f['packet_id']==packet for f in trace.since(mark))
        save('outside_qualified_scale_refused_before_position',refusal=refused)
        scale(2.);reset([.5,65.,.5]);mark=trace.mark();owner=admitted(request('mode_start',mode='survival'))
        inspect('scaled_new_owner',request('ticks',count=3),2.,mark)
        changed=scale(1.);scale(2.)
        stopped=until(lambda:r if (r:=request('record'))['status']['status']=='stopped' else None,5)
        assert stopped['scale_change_sequence']==changed['reading']['source']['sequence'],stopped
        assert request('wait',ms=100)==stopped
        stale=request('mode_keys',mode='survival',session_id=owner['session_id'],controls=keys(forward=1));assert not stale['admitted']
        save('changed_and_restored_scale_never_resumes_old_owner',record=stopped,first_change=changed,stale_refusal=stale)
        fresh=admitted(request('mode_start',mode='survival'));assert fresh['session_id']!=owner['session_id'] and fresh['scale_change_sequence'] is None
        stop('survival',fresh);save('explicit_scaled_restart',record=fresh)
    # Real death/respawn changes the world lifetime even in the same dimension.
    mode('survival');command('fill -4 65 -4 4 84 4 minecraft:air');reset([.5,65.,.5])
    old_world=request('player')['session'];owner=admitted(request('mode_start',mode='survival'))
    command('kill ClimbingProbe')
    until(lambda:r if (r:=request('record')) and r['status']['status']=='stopped' else None,5)
    request('respawn');until(lambda:request('player')['session']!=old_world,10)
    reset([.5,65.,.5])
    stale=request('mode_keys',mode='survival',session_id=owner['session_id'],controls=keys(forward=1));assert not stale['admitted'],stale
    if modern:
        # Force a fresh attribute receipt after the world reset, independently
        # of whether the server retained the old base or reset it to one.
        scale(.5);scale(1.)
    mark=trace.mark();fresh=admitted(request('mode_start',mode='survival'))
    assert fresh['session_id']!=owner['session_id'],fresh
    record=request('ticks',count=3);inspect('fresh_world_unit_model',record,1.,mark)
    stop('survival',fresh);save('world_change_rejects_old_scaled_owner',old_world=old_world,new_world=request('player')['session'],old_owner=owner,stale_refusal=stale)
    trace.expect_disconnect();request('disconnect')


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--binary',type=Path,required=True);p.add_argument('--jars',type=Path,required=True);p.add_argument('--version',choices=['1.16.1','1.21.11']);p.add_argument('--compiled-sdk-revision',required=True);a=p.parse_args()
    binary=a.binary.resolve();revision=subprocess.check_output(['git','rev-parse','HEAD'],cwd=REPO,text=True).strip();compiled=subprocess.check_output(['git','rev-parse',a.compiled_sdk_revision],cwd=REPO,text=True).strip()
    paths=['src','examples','Cargo.toml','Cargo.lock','data']
    assert not subprocess.check_output(['git','diff',compiled,'HEAD','--',*paths],cwd=REPO)
    assert not subprocess.check_output(['git','diff','HEAD','--',*paths],cwd=REPO)
    artifacts=['Cargo.lock','scripts/run_player_scale_live.py','scripts/run_player_scale_oracle.py','scripts/movement_oracle/player_scale_scenarios.json','scripts/movement_oracle/run.py','scripts/movement_oracle/java_1_16_1/MovementOracle.java','scripts/movement_oracle/java_1_21_11/MovementOracle.java','scripts/run_climbing_control.py','scripts/run_common_native.py','examples/climbing_control_probe.rs','data/client_api/player_scale_oracle.json.gz']
    sdk=dict(source_revision=revision,binary_build_revision=compiled,binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest(),artifacts={f:hashlib.sha256((REPO/f).read_bytes()).hexdigest() for f in artifacts})
    for version in [a.version] if a.version else ['1.16.1','1.21.11']:
        assert hashlib.sha256(binary.read_bytes()).hexdigest()==sdk['binary_sha256']
        run(version,binary,a.jars.resolve(),check=functools.partial(check,sdk=sdk))
        assert hashlib.sha256(binary.read_bytes()).hexdigest()==sdk['binary_sha256']
if __name__=='__main__':main()
