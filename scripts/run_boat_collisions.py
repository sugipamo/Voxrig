#!/usr/bin/env python3
"""Rigid boat/cart collision inputs on unchanged official loopback servers."""
import argparse
import functools
import gzip
import hashlib
import math
import copy
from pathlib import Path
import re
import struct
import subprocess
from concurrent.futures import ThreadPoolExecutor
from run_climbing_control import REPO, run
from run_common_native import NativeSocialReader, until


def wire_targets(version, peers):
    """Independent pinned wire codec: original targets, never render positions."""
    old=version=='1.16.1';base={};targets={}
    for sequence,p in enumerate(peers,1):
        if p['phase']!='play':continue
        kind=p['packet_id']
        if kind in ({0x25,0x3a} if old else {0x30,0x50}):base.clear();continue
        if kind not in ({0x00,0x28,0x29,0x56} if old else {0x01,0x33,0x34,0x23,0x7b}):continue
        r=NativeSocialReader(p,version);native_id=r.integer()
        if kind==(0x00 if old else 0x01):
            r.take(16);r.integer();pos=list(struct.unpack('>ddd',r.take(24)));base[native_id]=pos
        elif kind in ({0x28,0x29} if old else {0x33,0x34}):
            delta=struct.unpack('>hhh',r.take(6));prior=base.get(native_id)
            if prior is None:continue
            pos=[(math.floor(a*4096)+d)/4096 if old else ((math.floor(a*4096+.5)+d)/4096 if d else a) for a,d in zip(prior,delta)]
            base[native_id]=pos
        elif kind==(0x56 if old else 0x23):
            pos=list(struct.unpack('>ddd',r.take(24)));base[native_id]=pos
        else:
            pos=list(struct.unpack('>ddd',r.take(24)));r.take(24+8);flags=int.from_bytes(r.take(4),'big')
            if flags&7:continue
        targets[sequence]=(native_id,pos)
    return targets


def verify(version,record,trace):
    peers=[p for p in trace.since(0) if p['direction']=='clientbound' and p['phase'] in ('configuration','play')]
    targets=wire_targets(version,peers)
    samples=record['boat_motion']['collision_samples'];frames=record['boat_motion']['frames']
    if len(samples)!=record['attempted_tick'] or len(frames)!=record['dispatched_ticks']:raise RuntimeError('attempted collision samples differ from local dispatch history')
    oracle=gzip.open(REPO/'data/client_api/boat_collision_oracle.json.gz')
    original=__import__('json').load(oracle)['results'][version]
    dimensions={}
    for typ,scene in [('boat','rigid_front_boat'),('minecart','rigid_front_cart')]:
        box=next(r for r in original if r['name']==scene)['collision_boxes'][0]
        dimensions[typ]=(float(box[3])-float(box[0]),float(box[4])-float(box[1]))
    proofs=[]
    for index,sample in enumerate(samples,1):
        if sample['sampled_before_tick']!=index or len(sample['bodies'])>32:raise RuntimeError('collision budget/tick identity differs')
        source=sample['passengers']['source'];packet=peers[source['sequence']-1]
        r=NativeSocialReader(packet,version);vehicle=r.integer();passengers=[r.integer() for _ in range(r.integer())];r.end()
        if packet['packet_id']!=(0x4b if version=='1.16.1' else 0x69) or vehicle!=record['id']['mount']['vehicle_native_id'] or passengers!=sample['passengers']['value']:
            raise RuntimeError('excluded passengers differ from original native list')
        for body in sample['bodies']:
            entity=body['entity'];spawn=peers[entity['spawn_sequence']-1]
            if spawn['phase']!='play' or spawn['packet_id']!=(0 if version=='1.16.1' else 1):raise RuntimeError('collision lifetime is not an actual original spawn')
            r=NativeSocialReader(spawn,version)
            if r.integer()!=entity['native_id']:raise RuntimeError('collision lifetime changed numeric ID')
            receipt=body['received_position'];seq=receipt['source']['sequence']
            if receipt['source']['kind']!='received' or not entity['spawn_sequence']<=seq<=sample['receive_sequence']:
                raise RuntimeError('collision target outside owned receipt boundary')
            if targets.get(seq)!=(entity['native_id'],receipt['value']['position']):raise RuntimeError('collision target differs from original native wire codec')
            if entity['session']!=record['id']['mount']['session'] or entity['native_id'] in passengers:raise RuntimeError('collision input includes a foreign world or actual passenger')
            width,height=dimensions['minecart' if body['type_name'].endswith('minecart') else 'boat'];x,y,z=receipt['value']['position']
            if body['model_box']!=[x-width/2,y,z-width/2,x+width/2,y+height,z+width/2]:raise RuntimeError('received rigid target did not use original native dimensions')
            proofs.append(dict(sampled_before_tick=index,entity=entity,source=seq,original_target=peers[seq-1]))
    return proofs


def check(version,command,request,trace,report,*,sdk):
    report['sdk']=sdk;old=version=='1.16.1';boat='minecraft:boat' if old else 'minecraft:oak_boat'
    def native(tag,field='Pos'):
        value=command(f'data get entity @e[tag={tag},limit=1] {field}')
        m=re.search(r'\[([^]]+)\]',value)
        if not m:raise RuntimeError('original native vector unavailable: '+value)
        return [float(v.strip().rstrip('df')) for v in m[1].split(',')]
    def entity_types():return request('entities')['entities']
    def original_frames(record,boundary):
        frames=record['boat_motion']['frames']
        def received():
            packets=trace.since(boundary)
            moves=[p for p in packets if p['direction']=='serverbound' and p['phase']=='play' and p['packet_id']==(0x16 if old else 0x21)]
            paddles=[p for p in packets if p['direction']=='serverbound' and p['phase']=='play' and p['packet_id']==(0x17 if old else 0x22)]
            return (moves,paddles) if len(moves)>=len(frames) and len(paddles)>=len(frames) else None
        moves,paddles=until(received,5) if frames else ([],[])
        if len(moves)!=len(frames) or len(paddles)!=len(frames):raise RuntimeError('collision movement writes differ from completed tick count')
        for f,m,p in zip(frames,moves,paddles):
            wire=struct.pack('>dddff',*f['position'],*f['rotation'])+(bytes([f['on_ground']]) if not old else b'')
            if m['body_hex']!=wire.hex() or p['body_hex']!=bytes(f['paddles']).hex():raise RuntimeError('original movement differs from retained collision model')
        return moves+paddles
    for kind in ('boat','minecart'):
        command('kill @e[tag=CollisionMount]');command('kill @e[tag=CollisionOther]')
        request('wait',ms=150)
        command('fill -8 63 -8 8 72 8 minecraft:air');command('fill -8 63 -8 8 64 8 minecraft:stone')
        command('fill -3 65 5 3 68 5 minecraft:stone')
        command('tp ClimbingProbe 0.5 65 -0.5 0 0');request('prepare',position=[.5,65.,-.5])
        command('summon '+boat+' 0.5 65.25 1.5 {Tags:["CollisionMount"],Invulnerable:1b}')
        mount=request('mount',type=boat)
        other=boat if kind=='boat' else 'minecraft:minecart'
        command('summon '+other+' 0.5 65.25 4 {Tags:["CollisionOther"],NoGravity:1b,Invulnerable:1b}')
        until(lambda:sum(e['motion']['entity']['type_name']==other for e in entity_types())>=(2 if kind=='boat' else 1),5)
        if 'Test passed' not in command('execute if entity @e[tag=CollisionOther,type='+other+']'):raise RuntimeError('independent native vehicle type differs')
        request('wait',ms=150);boundary=trace.mark()
        inputs=[dict(forward=1,strafe=0,jump=False)]*35+[dict(forward=0,strafe=0,jump=False)]*5
        result=request('drive_cancelled_waiter',inputs=inputs) if kind=='boat' else dict(record=request('drive',inputs=inputs))
        record=result['record']
        if record['stage']!='submitted' or record['dispatched_ticks']!=40:raise RuntimeError('rigid collision finite plan did not completely submit')
        proofs=verify(version,record,trace);writes=original_frames(record,boundary)
        frames=record['boat_motion']['frames']
        if not any(f['velocity'][2]==0. and f['paddles']==[True,True] and f['position'][2]<3.4 for f in frames):
            raise RuntimeError('rigid entity did not stop the forward prediction before the rear block wall')
        final=frames[-1]['position']
        def endpoint():
            p=native('CollisionMount')
            return p if max(abs(a-b) for a,b in zip(p,final))<.4 else None
        position=until(endpoint,5)
        report['checks'].append(dict(name=kind+'_native_rigid_collision_and_dispatch',mount=mount,outcome=result,proofs=proofs,writes=writes,native_position=position,native_other=native('CollisionOther')))
        boundary=trace.mark();neutral=request('drive',inputs=[dict(forward=0,strafe=0,jump=False)]*8)
        expected=copy.deepcopy(frames[-1])
        current_velocity=neutral['boat_motion']['received_velocity'];prior_velocity=record['boat_motion']['received_velocity']
        if current_velocity['source']['sequence']>prior_velocity['source']['sequence']:expected['velocity']=current_velocity['value']
        if neutral['boat_motion']['initial_frame']!=expected:raise RuntimeError('neutral continuation discarded the completed collision model or fresh original velocity')
        report['checks'].append(dict(name=kind+'_owned_neutral_continuation',record=neutral,proofs=verify(version,neutral,trace),writes=original_frames(neutral,boundary)))
        old_sources=__import__('json').dumps(record,sort_keys=True)
        command('kill @e[tag=CollisionOther]');request('wait',ms=150)
        if __import__('json').dumps(record,sort_keys=True)!=old_sources:raise RuntimeError('removed entity rewrote saved collision history')
        if kind=='boat':
            boundary=trace.mark();after=request('drive',inputs=[dict(forward=0,strafe=0,jump=False)]*4)
            if any(s['bodies'] for s in after['boat_motion']['collision_samples']):raise RuntimeError('removed collision body carried into later live ticks')
            report['checks'].append(dict(name='original_despawn_retires_future_collision_inputs',record=after,proofs=verify(version,after,trace),writes=original_frames(after,boundary)))
        else:
            p=native('CollisionMount');command(f'summon minecraft:sheep {p[0]+1} {p[1]} {p[2]} {{Tags:["CollisionUnsupported"],NoAI:1b,NoGravity:1b}}')
            until(lambda:any(e['motion']['entity']['type_name']=='minecraft:sheep' for e in entity_types()),5)
            boundary=trace.mark();failed=request('drive_until_interrupted',inputs=[dict(forward=1,strafe=0,jump=False),dict(forward=0,strafe=0,jump=False)])
            if failed['record']['stage']!='requires_inspection' or failed['record']['dispatched_ticks'] or 'not audited' not in failed['record']['requires_inspection']:
                raise RuntimeError('unsupported nearby pose did not refuse before dispatch')
            if any(p['direction']=='serverbound' and p['phase']=='play' and p['packet_id'] in ((0x16,0x17) if old else (0x21,0x22)) for p in trace.since(boundary)):
                raise RuntimeError('unsupported collision sent an attempted boat frame')
            again=request('drive_until_interrupted',inputs=[dict(forward=0,strafe=0,jump=False)])
            if again['record']!=failed['record']:raise RuntimeError('unresolved collision was replayed or replaced')
            report['checks'].append(dict(name='unsupported_pose_refusal_and_no_automatic_replay',first=failed,rejected=again))
            command('kill @e[tag=CollisionUnsupported]')
        if kind=='boat':
            dismount=request('dismount')
            if dismount['stage']!='completed':raise RuntimeError('post-collision received dismount did not complete')
            report['checks'].append(dict(name=kind+'_post_collision_received_dismount',record=dismount))
        else:
            # The retained unresolved operation deliberately blocks SDK motion
            # and dismount; never weaken that fence or manufacture a recovery.
            boundary=trace.mark();command('tp ClimbingProbe 0.5 65 -0.5 0 0')
            until(lambda:'Test passed' in command('execute unless entity @a[name=ClimbingProbe,nbt={RootVehicle:{}}]'),5)
            def original_exit():
                for p in trace.since(boundary):
                    if p['direction']=='clientbound' and p['phase']=='play' and p['packet_id']==(0x4b if old else 0x69):
                        r=NativeSocialReader(p,version);target=r.integer();members=[r.integer() for _ in range(r.integer())];r.end()
                        if target==record['id']['mount']['vehicle_native_id'] and record['vehicle']['player_native_id'] not in members:return p
                return None
            exited=until(original_exit,5)
            retained=request('vehicle_record')
            if retained!=failed['record']:raise RuntimeError('forced exit rewrote the original refusal record')
            report['checks'].append(dict(name='actual_forced_exit_retains_unresolved_collision_without_replay',record=retained,original_passengers=exited))
    trace.expect_disconnect();closed=request('vehicle_disconnect')
    if closed['record'] is None:raise RuntimeError('closed connection lost collision history')
    report['checks'].append(dict(name='collision_history_readable_after_close',record=closed))


def check_nested_attachment(version, command, request, trace, report, *, sdk):
    """Original modern /ride can attach then restore a mounted boat externally."""
    if version != '1.21.11':
        raise RuntimeError('legacy original server has no /ride command')
    report['sdk'] = sdk
    command('fill -8 63 -8 8 72 8 minecraft:air')
    command('fill -8 63 -8 8 64 8 minecraft:stone')
    command('tp ClimbingProbe 0.5 65 -0.5 0 0')
    request('prepare', position=[.5, 65., -.5])
    command('summon minecraft:oak_boat 0.5 65.25 1.5 {Tags:["CollisionMount"],Invulnerable:1b}')
    mount = request('mount', type='minecraft:oak_boat')
    native_mount = mount['relation']['value']['mount']
    command('summon minecraft:minecart 0.5 65.25 2 {Tags:["CollisionParent"],NoGravity:1b,Invulnerable:1b}')
    def received_parent():
        entities = request('entities')['entities']
        return entities if any(e['motion']['entity']['type_name'] == 'minecraft:minecart'
                               for e in entities) else None
    entities = until(received_parent, 5)
    parent = next(e['motion']['entity']['id']['native_id'] for e in entities
                  if e['motion']['entity']['type_name'] == 'minecraft:minecart')
    request('wait', ms=150)
    boundary = trace.mark()

    def change_attachment():
        until(lambda: any(p['direction'] == 'serverbound' and p['phase'] == 'play'
                          and p['packet_id'] == 0x21 for p in trace.since(boundary)), 5)
        attached = command('ride @e[tag=CollisionMount,limit=1] mount @e[tag=CollisionParent,limit=1]')
        if 'started riding' not in attached:
            raise RuntimeError('original /ride did not actually attach the vehicle: ' + attached)
        # Two commands in one original server tick can coalesce before its
        # entity tracker sends SET_PASSENGERS. Test actual receipt, not an
        # unobservable intermediate server state or the command response.
        def original_attached_list():
            for packet in trace.since(boundary):
                if packet['direction'] != 'clientbound' or packet['phase'] != 'play' or packet['packet_id'] != 0x69:
                    continue
                reader = NativeSocialReader(packet, version)
                target = reader.integer()
                members = [reader.integer() for _ in range(reader.integer())]
                reader.end()
                if target == parent and native_mount['vehicle_native_id'] in members:
                    return packet
            return None
        until(original_attached_list, 5)
        restored = command('ride @e[tag=CollisionMount,limit=1] dismount')
        if 'stopped riding' not in restored:
            raise RuntimeError('original /ride did not restore the attachment: ' + restored)

    with ThreadPoolExecutor(max_workers=1) as executor:
        change = executor.submit(change_attachment)
        outcome = request('drive_until_interrupted', inputs=[dict(forward=0, strafe=0, jump=False)] * 120)
        change.result(timeout=10)
    record = outcome['record']
    if record['stage'] != 'requires_inspection' or not 0 < record['dispatched_ticks'] < 120:
        raise RuntimeError('actual nested attachment did not fence a running finite owner')

    def original_attachment():
        peers = [p for p in trace.since(0) if p['direction'] == 'clientbound'
                 and p['phase'] in ('configuration', 'play')]
        changes = []
        for sequence, packet in enumerate(peers, 1):
            if packet['phase'] != 'play' or packet['packet_id'] != 0x69:
                continue
            reader = NativeSocialReader(packet, version)
            target = reader.integer()
            members = [reader.integer() for _ in range(reader.integer())]
            reader.end()
            if target == parent:
                changes.append(dict(sequence=sequence, members=members, original=packet))
        if any(native_mount['vehicle_native_id'] in c['members'] for c in changes) and any(
                not c['members'] for c in changes):
            return changes
        return None

    original = until(original_attachment, 5)
    vehicle = request('vehicle')
    attached = next(c for c in original if native_mount['vehicle_native_id'] in c['members'])
    if vehicle['attachment_change_sequence'] != attached['sequence']:
        raise RuntimeError('nested fence source differs from the original received passenger packet')
    again = request('drive_until_interrupted', inputs=[dict(forward=0, strafe=0, jump=False)])
    if again['record'] != record:
        raise RuntimeError('restored attachment automatically replayed or replaced the failed owner')
    boundary = trace.mark()
    request('wait', ms=200)
    if any(p['direction'] == 'serverbound' and p['phase'] == 'play' and p['packet_id'] in (0x21, 0x22)
           for p in trace.since(boundary)):
        raise RuntimeError('terminal nested owner dispatched later boat frames')
    report['checks'].append(dict(name='original_nested_attachment_and_restore_fence_running_owner',
                                 outcome=outcome, vehicle=vehicle, original=original))
    report['checks'].append(dict(name='restored_nested_attachment_has_no_automatic_replay', rejected=again))
    trace.expect_disconnect()
    closed = request('vehicle_disconnect')
    if closed['record'] != record:
        raise RuntimeError('closing the nested owner rewrote its original retained failure')
    report['checks'].append(dict(name='nested_attachment_history_readable_after_close', record=closed))


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--accept-eula',action='store_true',required=True)
    p.add_argument('--version',choices=('1.16.1','1.21.11'),action='append')
    p.add_argument('--binary',type=Path,required=True)
    p.add_argument('--jars',type=Path,default=REPO/'.local/climbing/downloads')
    p.add_argument('--compiled-sdk-revision',required=True)
    p.add_argument('--nested-attachment-only', action='store_true', help='modern original /ride receipt fences a running owner; unavailable on legacy /ride')
    args=p.parse_args();revision=subprocess.check_output(['git','rev-parse','HEAD'],cwd=REPO,text=True).strip()
    if args.nested_attachment_only and args.version != ['1.21.11']:
        p.error('--nested-attachment-only requires --version 1.21.11; legacy has no original /ride command')
    compiled=subprocess.check_output(['git','rev-parse',args.compiled_sdk_revision],cwd=REPO,text=True).strip()
    if subprocess.check_output(['git','diff',compiled,'HEAD','--','src','examples','Cargo.toml','Cargo.lock'],cwd=REPO):raise RuntimeError('compiled SDK differs from runtime source')
    files=('Cargo.lock','scripts/run_boat_collisions.py','scripts/run_climbing_control.py','scripts/run_common_native.py','examples/climbing_control_probe.rs','scripts/movement_oracle/run.py','scripts/movement_oracle/boat_collision_scenarios.json','scripts/movement_oracle/java_1_16_1/MovementOracle.java','scripts/movement_oracle/java_1_21_11/MovementOracle.java','data/client_api/boat_collision_oracle.json.gz')
    sdk=dict(source_revision=revision,binary_build_revision=compiled,binary_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest(),source_diff_sha256=hashlib.sha256(subprocess.check_output(['git','diff','HEAD'],cwd=REPO)).hexdigest(),artifacts={f:hashlib.sha256((REPO/f).read_bytes()).hexdigest() for f in files})
    for version in args.version or ('1.16.1','1.21.11'):
        selected = check_nested_attachment if args.nested_attachment_only else check
        run(version,args.binary.resolve(),args.jars.resolve(),check=functools.partial(selected,sdk=sdk))
if __name__=='__main__':main()
