#!/usr/bin/env python3
"""Qualify owned walking/boat contacts on unchanged official servers."""
import argparse
import functools
import hashlib
import json
import os
import queue
import threading
import re
import subprocess
from pathlib import Path
from run_climbing_control import REPO, run
from run_common_native import until, PacketTraceProxy, pump


def peer_checks(version,command,request,trace,report,binary):
    folder=Path(trace.log.name).parent
    proxy=PacketTraceProxy(trace.server_port,version,folder/'owner-peer-packets.jsonl')
    peer=None
    try:
        with (folder/'owner-peer.log').open('w') as output,(folder/'owner-peer.stderr.log').open('w') as stderr:
            peer=subprocess.Popen([str(binary)],cwd=REPO,env=dict(os.environ,VOXRIG_MINECRAFT_VERSION=version,VOXRIG_PORT=str(proxy.port),VOXRIG_PROBE_USERNAME='ContactPeer'),stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=stderr,text=True,bufsize=1)
            messages=queue.Queue();threading.Thread(target=pump,args=(peer.stdout,messages,output),daemon=True).start()
            assert json.loads(messages.get(timeout=20))=={'ready':True}
            def ask(action,**arguments):
                peer.stdin.write(json.dumps(dict(command=action,**arguments))+'\n');peer.stdin.flush();return json.loads(messages.get(timeout=20))
            command('tp ContactPeer .5 65 .7 0 0');ask('prepare',position=[.5,65.,.7])
            remote=until(lambda:next((e for e in request('entities')['entities'] if e['motion']['entity']['type_name']=='minecraft:player'),None),10)
            identity=remote['motion']['entity']['id']
            request('start');stationary=request('ticks',count=2)
            assert stationary['status']['status']=='running'
            assert any(b['entity']==identity for b in stationary['entity_contacts']['bodies'])
            assert max(abs(a-b) for a,b in zip(stationary['frame']['position'],[.5,65.,.5]))<1e-8,stationary
            ask('start');ask('keys',controls=dict(forward=1,strafe=0,jump=False,sneak=False,sprint=False,yaw=0.,pitch=0.))
            moving=request('ticks',count=5);assert moving['status']['status']=='running'
            assert moving['entity_contacts']['applied_record']>stationary['entity_contacts']['applied_record']
            ask('stop');request('stop')
            report['checks'].append(dict(name='actual_remote_player_constructor_and_motion',entity=remote,stationary=stationary,moving=moving))
            proxy.expect_disconnect();ask('disconnect');assert peer.wait(timeout=10)==0
            until(lambda:not any(e['motion']['entity']['id']==identity for e in request('entities')['entities']),10)
            report['peer_trace_errors']=proxy.errors;assert not proxy.errors
    finally:
        if peer and peer.poll() is None:peer.terminate();peer.wait(timeout=10)
        proxy.close()


def check(version,command,request,trace,report,binary,sdk,failure_scenario):
    report['sdk']=sdk
    modern=version=='1.21.11'
    def entities():return request('entities')['entities']
    def cow():return next((e for e in entities() if e['motion']['entity']['type_name']=='minecraft:cow'),None)
    def reset(position):
        command('tp ClimbingProbe '+' '.join(map(str,position))+' 0 0')
        request('prepare',position=position)
    def native(selector,field):
        text=command(f'data get entity {selector} {field}')
        match=re.search(r'\[([^]]+)\]',text)
        assert match,text
        return [float(v.strip().removesuffix('d').removesuffix('f')) for v in match[1].split(',')]
    def source_frame(sequence):
        frames=[f for f in trace.since(0) if f['direction']=='clientbound' and f['phase'] in ('configuration','play')]
        return frames[sequence-1]
    def record(name,entry):report['checks'].append(dict(name=name,**entry))
    def contact_frame(name,value,expect_pairs=None):
        assert value['status']['status']=='running',value
        frame=value['entity_contacts'];assert frame and frame['model_tick']>0,value
        if expect_pairs is not None:assert (frame['pair_calls']>0)==expect_pairs,frame
        context=request('entities')['session']
        for body in frame['bodies']:
            assert body['entity']['session']==context
        actual=native('ClimbingProbe','Pos')
        assert max(abs(a-b) for a,b in zip(actual,value['frame']['position']))<.5,(actual,value)
        record(name,dict(record=value,native_position=actual))
        return frame
    command('kill @e[type=!minecraft:player]')
    command('difficulty normal')
    # Fixture cleanup must not introduce unqualified item-entity dispatch.
    command('gamerule '+('minecraft:mob_drops' if modern else 'doMobLoot')+' false')
    command('gamerule '+('minecraft:block_drops' if modern else 'doTileDrops')+' false')
    reset([.5,65.,.5])
    peer_checks(version,command,request,trace,report,binary)
    reset([.5,65.,.5])
    command('summon minecraft:cow .9 65 .5 {Tags:["OwnerCow"],NoAI:1b,NoGravity:1b,Silent:1b,Invulnerable:1b}')
    received=until(cow,10);spawn=received['motion']['entity'];ident=spawn['id']
    initial=source_frame(spawn['spawn_position']['source']['sequence'])
    record('actual_spawn_origin',dict(spawn=spawn,original_spawn=initial,native_position=native('@e[tag=OwnerCow,limit=1]','Pos')))
    request('start')
    first=contact_frame('walking_contact',request('ticks',count=2),True)
    request('stop')
    command('team add ownerquiet');command('team modify ownerquiet collisionRule never')
    command('team join ownerquiet @e[tag=OwnerCow]')
    until(lambda:any(t['name']=='ownerquiet' and t['parameters']['value']['collision']=='never' and t['members'] for t in request('teams')['teams']),10)
    reset([.5,65.,.5]);request('start')
    contact_frame('received_team_excludes_contact',request('ticks',count=2),False)
    request('stop');command('team leave @e[tag=OwnerCow]')
    # Original living sleep-position metadata, separate from the pose enum.
    command('setblock 0 65 0 minecraft:red_bed[part=head,facing=south]')
    command('setblock 0 65 -1 minecraft:red_bed[part=foot,facing=south]')
    command('data merge entity @e[tag=OwnerCow,limit=1] '+ ('{sleeping_pos:[I;0,65,0]}' if modern else '{SleepingX:0,SleepingY:65,SleepingZ:0}'))
    reading=until(lambda:next((e for e in request('entity_sleeping') if e['entity']['id']==ident and e['sleeping'] and e['sleeping']['sleeping']),None),10)
    assert reading['sleeping']['position']==dict(x=0,y=65,z=0),reading
    sleep_receipt=source_frame(reading['sleeping']['source']['sequence'])
    reset([.5,65.5625,.5]);request('start')
    sleeping=contact_frame('sleep_position_receipt_and_model',request('ticks',count=2))
    assert any(b['entity']==ident for b in sleeping['bodies'])
    # AI-step interpolation can move numerical feet after the sleep callback;
    # the native oracle qualifies this order, rather than assuming bed feet.
    record('received_sleep_position',dict(reading=reading,original_metadata=sleep_receipt))
    request('stop')
    command('setblock 0 65 0 minecraft:air')
    command('setblock 0 65 -1 minecraft:air')
    command('tp @e[tag=OwnerCow,limit=1] .9 65 .5')
    until(lambda:next((e for e in request('entity_sleeping') if e['entity']['id']==ident and e['sleeping'] and not e['sleeping']['sleeping']),None),10)
    reset([.5,65.,.5]);request('start')
    contact_frame('sleep_cleared_new_origin',request('ticks',count=2),True)
    # Real received target change, model position retained independently.
    mark=trace.mark();command('tp @e[tag=OwnerCow,limit=1] 2 65 .5')
    moved=contact_frame('received_move_during_owner',request('ticks',count=4))
    assert moved['applied_record']>first['applied_record']
    record('motion_receipts',dict(frames=trace.since(mark),native_position=native('@e[tag=OwnerCow,limit=1]','Pos')))
    request('stop')
    command('kill @e[tag=OwnerCow]')
    until(lambda:not any(e['motion']['entity']['id']==ident for e in entities()),10)
    command('summon minecraft:cow .9 65 .5 {Tags:["OwnerCow"],NoAI:1b,NoGravity:1b,Silent:1b,Invulnerable:1b}')
    fresh=until(lambda:(e if e and e['motion']['entity']['id']!=ident else None) if (e:=cow()) else None,10)
    reset([.5,65.,.5]);request('start')
    frame=contact_frame('removed_fresh_spawn_origin',request('ticks',count=2),True)
    assert all(b['entity']!=ident for b in frame['bodies'])
    record('new_spawn_lifetime',dict(entity=fresh['motion']['entity']))
    # Corrections terminally discard the remote origin, then explicit restart.
    command('tp ClimbingProbe 3.5 65 3.5 0 0')
    terminal=until(lambda:r if (r:=request('wait',ms=100))['status']['status']=='stopped' else None,10)
    record('own_correction_discards_contact_owner',dict(record=terminal))
    request('prepare',position=[3.5,65.,3.5]);request('start');contact_frame('explicit_new_owner',request('ticks',count=2));request('stop')
    # A boat receives living boxes at its native slot; ordinary push stays separate.
    command('tp @e[tag=OwnerCow,limit=1] .5 65 2')
    reset([.5,65.,.5])
    boat='minecraft:oak_boat' if modern else 'minecraft:boat'
    command(f'summon {boat} .5 65 .5 {{Tags:["OwnerBoat"]}}')
    request('wait',ms=200);request('mount',type=boat)
    inputs=[dict(forward=1,strafe=0,jump=False) for _ in range(18)]+[dict(forward=0,strafe=0,jump=False) for _ in range(6)]
    mark=trace.mark();driven=request('drive_cancelled_waiter',inputs=inputs)
    result=driven['record'];assert result['stage']=='submitted',driven
    samples=result['boat_motion']['collision_samples'];assert len(samples)==len(inputs)
    living=[b for s in samples for b in s['bodies'] if b['geometry']['kind']=='living_interpolation']
    assert living,(samples,driven)
    for s in samples:
        assert all(b['entity']['native_id'] not in s['passengers']['value'] for b in s['bodies'])
    packet=0x21 if modern else 0x16;paddle=0x22 if modern else 0x17
    # The asynchronous trace decoder can trail the completed native writer.
    until(lambda:sum(f['direction']=='serverbound' and f['phase']=='play' and f['packet_id']==paddle for f in trace.since(mark))>=len(inputs),5)
    frames=trace.since(mark)
    moves=[f for f in frames if f['direction']=='serverbound' and f['phase']=='play' and f['packet_id']==packet]
    paddles=[f for f in frames if f['direction']=='serverbound' and f['phase']=='play' and f['packet_id']==paddle]
    assert len(moves)==len(inputs) and len(paddles)==len(inputs),(len(moves),len(paddles))
    import struct
    for predicted,wire in zip(result['boat_motion']['frames'],moves):
        body=struct.pack('>dddff',*predicted['position'],*predicted['rotation'])+(bytes([predicted['on_ground']]) if modern else b'')
        assert wire['body_hex']==body.hex()
    assert paddles[-1]['body_hex']=='0000'
    record('boat_living_geometry_and_cancelled_waiter',dict(record=result,frames=frames,native_position=native('@e[tag=OwnerBoat,limit=1]','Pos')))
    exited=request('dismount');assert exited['stage']=='completed';record('actual_dismount',dict(record=exited))
    # Force an actual server-owned dismount during a still-pending finite plan.
    command('tp @e[tag=OwnerBoat,limit=1] .5 65 .5')
    reset([.5,65.,.5]);request('mount',type=boat)
    interruption=[]
    def eject():
        interruption.append(command('kill @e[tag=OwnerBoat]' if failure_scenario=='dismount' else 'summon minecraft:armor_stand 1 65 1 {Tags:["UnsupportedOwner"],NoGravity:1b}'))
    timer=threading.Timer(.5,eject);mark=trace.mark();timer.start()
    try:
        interrupted=request('drive_until_interrupted',inputs=[dict(forward=1,strafe=0,jump=False) for _ in range(59)]+[dict(forward=0,strafe=0,jump=False)])
    finally:timer.join(timeout=10)
    assert interruption and interrupted['error'],interrupted
    failed=interrupted['record'];assert failed['stage']=='requires_inspection',failed
    assert failed['dispatched_ticks']<60 and failed['failure_release'],failed
    released=failed['failure_release']['attempted']
    if failure_scenario=='unsupported':assert released and failed['failure_release']['sent'],failed
    # Removal can deliver own correction before passenger/removal packets. A
    # coherent still-mounted boundary then permits one neutral-only release.
    if released:assert failed['failure_release']['sent'],failed
    else:assert not failed['failure_release']['sent'] and failed['failure_release']['problem'],failed
    request('wait',ms=200);before=trace.since(mark);request('wait',ms=200);after=trace.since(mark)
    def boat_writes(frames):return [f for f in frames if f['direction']=='serverbound' and f['phase']=='play' and f['packet_id'] in (packet,paddle)]
    assert len(boat_writes(before))==len(boat_writes(after)),(before,after)
    if released:
        last_paddles=[f for f in boat_writes(after) if f['packet_id']==paddle]
        assert last_paddles[-1]['body_hex']=='0000'
    record('forced_dismount_stops_old_owner' if failure_scenario=='dismount' else 'unsupported_dispatch_releases_once',dict(record=failed,frames=after,server_command=interruption))
    trace.expect_disconnect();request('disconnect')


def main():
    p=argparse.ArgumentParser(description=__doc__);p.add_argument('--binary',type=Path,required=True);p.add_argument('--jars',type=Path,required=True);p.add_argument('--version',choices=['1.16.1','1.21.11']);p.add_argument('--compiled-sdk-revision',required=True);p.add_argument('--failure-scenario',choices=['dismount','unsupported'],default='dismount');a=p.parse_args()
    binary=a.binary.resolve();sha=hashlib.sha256(binary.read_bytes()).hexdigest()
    revision=subprocess.check_output(['git','rev-parse','HEAD'],cwd=REPO,text=True).strip()
    compiled=subprocess.check_output(['git','rev-parse',a.compiled_sdk_revision],cwd=REPO,text=True).strip()
    paths=['src','examples','Cargo.toml','Cargo.lock','data']
    assert not subprocess.check_output(['git','diff',compiled,'HEAD','--',*paths],cwd=REPO)
    assert not subprocess.check_output(['git','diff','HEAD','--',*paths],cwd=REPO)
    artifacts=['Cargo.lock','scripts/run_entity_owner_live.py','scripts/run_entity_owner_oracle.py','scripts/run_climbing_control.py','scripts/run_common_native.py','examples/climbing_control_probe.rs','data/client_api/entity_owner_oracle.json.gz']
    sdk=dict(source_revision=revision,binary_build_revision=compiled,binary_sha256=sha,artifacts={f:hashlib.sha256((REPO/f).read_bytes()).hexdigest() for f in artifacts})
    for version in [a.version] if a.version else ['1.16.1','1.21.11']:
        assert hashlib.sha256(binary.read_bytes()).hexdigest()==sha
        run(version,binary,a.jars.resolve(),check=functools.partial(check,binary=binary,sdk=sdk,failure_scenario=a.failure_scenario))
        assert hashlib.sha256(binary.read_bytes()).hexdigest()==sha
if __name__=='__main__':main()
