#!/usr/bin/env python3
"""Qualify explicit interpolation schedules seeded by real original motion receipts.

This is a numerical interpolation model, not native remote-client tick scheduling
or selected-owner collision/movement integration. Collision predicates are explicit.
"""
import argparse
import functools
import gzip
import hashlib
import json
from pathlib import Path
import struct
import subprocess
from run_climbing_control import REPO, run
from run_common_native import NativeSocialReader, until
from run_entity_pose_live import check as peer_pose_check


def check(version, command, request, trace, report, binary, sdk):
    report['sdk']=sdk
    peer_pose_check(version,command,request,trace,report,binary,sdk,disconnect_observer=False)
    def entities():return request('entities')['entities']
    command('summon minecraft:cow 0.5 70 5.5 {Tags:["InterpolationCow"],NoAI:1b,NoGravity:1b,Invulnerable:1b}')
    def cow():return next((e for e in entities() if e['motion']['entity']['type_name']=='minecraft:cow'),None)
    received=until(cow,10)['motion'];identity=received['entity']['id']
    initial=dict(position=received['position']['value']['position'],rotation=received['rotation']['value'])
    frame=request('interpolation_new',pose=initial,length=3)
    assert frame['pose']==initial and frame['remaining_steps']==0
    report['checks'].append(dict(name='explicit_model_at_original_spawn',identity=identity,seed=received,result=frame))
    # Keep each real motion receipt in packet order. The test schedules local
    # model steps explicitly and does not claim these are the native client's ticks.
    page=request('entity_history',tail=True);assert page['gap'] is None
    def pose_input(result):
        pose=dict(result['model']['Ok']['pose'])
        pose.update(position_bits=result['pose_bits']['position'],rotation_bits=result['pose_bits']['rotation'])
        return pose
    previous=received['position']['source']['sequence']
    for index,x in enumerate([16.5,-16.5,16.5]):
        command(f'tp @e[tag=InterpolationCow,limit=1] {x} 70 5.5 {90+index*45} 20')
        def moved():
            e=cow()
            if not e:return None
            m=e['motion'];p=m['position']
            return m if p and p['source']['sequence']>previous and p['value']['position']==[x,70.,5.5] else None
        motion=until(moved,10);sequence=motion['position']['source']['sequence']
        assert motion['rotation']['source']['sequence']==sequence
        packets=[p for p in trace.since(0) if p['direction']=='clientbound' and p['phase'] in ('configuration','play')]
        original=packets[sequence-1];r=NativeSocialReader(original,version)
        assert r.integer()==identity['native_id']
        native_position=list(struct.unpack('>ddd',r.take(24)))
        assert native_position==motion['position']['value']['position']
        if version=='1.16.1':
            assert original['packet_id']==0x56,original
            rotation=[n*360/256 for n in struct.unpack('>bb',r.take(2))]
        else:
            # Modern absolute sync and flags-free teleport both carry velocity
            # before the original body float rotation.
            assert original['packet_id'] in (0x23,0x7b),original
            r.take(24);rotation=list(struct.unpack('>ff',r.take(8)))
        assert rotation==motion['rotation']['value']
        target=dict(position=native_position,rotation=rotation)
        result=request('interpolation_target',pose=target);assert 'Ok' in result['model']
        frame=result['model']['Ok'];assert frame['remaining_steps']==3
        page=request('entity_history');assert page['gap'] is None and not page['has_more']
        records=[v for v in page['records'] if v['receive_sequence']==sequence and 'Motion' in v['kind']]
        assert len(records)==1 and records[0]['kind']['Motion']['entity']['id']==identity
        assert records[0]['kind']['Motion']['position']==motion['position']
        report['checks'].append(dict(name='original_motion_target_'+str(index),receipt=motion,original_packet=original,history=records[0],result=result))
        first=request('interpolation_step',pose=pose_input(result),**({'collision_free':True} if version=='1.21.11' else {}))
        assert 'Ok' in first['model'] and first['model']['Ok']['remaining_steps']==2
        frame=first['model']['Ok']
        duplicate=request('interpolation_target',pose=target)
        expected=2 if version=='1.21.11' else 3
        assert duplicate['model']['Ok']['remaining_steps']==expected
        report['checks'].append(dict(name='version_bound_duplicate_target_'+str(index),first=first,duplicate=duplicate))
        frame=duplicate['model']['Ok']
        if version=='1.21.11':
            refusal=request('interpolation_step',pose=pose_input(duplicate))
            assert refusal['model']=={'Err':'MissingCollisionQuery'} and refusal['retained']==frame
            report['checks'].append(dict(name='missing_collision_query_preserves_model_'+str(index),result=refusal))
        continuation=duplicate
        while frame['remaining_steps']:
            continuation=request('interpolation_step',pose=pose_input(continuation),**({'collision_free':True} if version=='1.21.11' else {}))
            frame=continuation['model']['Ok']
        assert frame['pose']['position']==target['position'] and frame['target'] is None
        assert (frame['pose']['rotation'][0]-target['rotation'][0])%360==0
        assert frame['pose']['rotation'][1]==target['rotation'][1]
        # Numerical continuation does not mutate the received latest target.
        current=cow()['motion'];assert current['position']==motion['position']
        report['checks'].append(dict(name='explicit_model_steps_leave_receipts_unchanged_'+str(index),result=frame))
        previous=sequence
    # Also compare original native scheduled results over this live API connection.
    native=json.loads(gzip.decompress((REPO/'data/client_api/entity_interpolation_oracle.json.gz').read_bytes()))['versions'][version]['results']
    for scenario in [0,1,4,6,13,34]:
        row=next(v for v in native if v['length']==3 and v['scenario']==scenario)
        request('interpolation_new',pose=row['initial'],length=3)
        for event in row['events']:
            op=event['operation'];args={}
            if op!='cancel':args['pose']=event['input']
            if 'before' in event:args['before']=event['before']
            if 'collision_free' in event:args['collision_free']=event['collision_free']
            result=request('interpolation_'+op,**args)
            assert 'Ok' in result['model'],result
            expected=event['pose']
            assert result['pose_bits']==dict(position=expected['position_bits'],rotation=expected['rotation_bits']),(version,scenario,event,result)
            assert result['model']['Ok']['remaining_steps']==event['remaining_steps']
        report['checks'].append(dict(name='original_numerical_schedule_'+str(scenario),operations=len(row['events'])))
    command('kill @e[tag=InterpolationCow]');until(lambda:not cow(),10)
    canceled=request('interpolation_cancel');assert canceled['model']['Ok']['remaining_steps']==0
    report['checks'].append(dict(name='explicit_cancel_after_original_removal',retired=identity,result=canceled))
    trace.expect_disconnect();request('disconnect')


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--accept-eula',action='store_true',required=True)
    p.add_argument('--binary',type=Path,required=True)
    p.add_argument('--jars',type=Path,default=REPO/'.local/climbing/downloads')
    p.add_argument('--version',choices=['1.16.1','1.21.11'],action='append')
    p.add_argument('--compiled-sdk-revision',required=True)
    args=p.parse_args();revision=subprocess.check_output(['git','rev-parse','HEAD'],text=True,cwd=REPO).strip()
    compiled=subprocess.check_output(['git','rev-parse',args.compiled_sdk_revision],text=True,cwd=REPO).strip()
    paths=['src','examples','Cargo.toml','Cargo.lock','data']
    assert not subprocess.check_output(['git','diff',compiled,'HEAD','--',*paths],cwd=REPO)
    assert not subprocess.check_output(['git','diff','HEAD','--',*paths],cwd=REPO)
    artifacts=['Cargo.lock','scripts/run_entity_interpolation_live.py','scripts/run_entity_interpolation_oracle.py','scripts/run_entity_pose_live.py','scripts/run_climbing_control.py','scripts/run_common_native.py','examples/climbing_control_probe.rs','data/client_api/entity_interpolation_oracle.json.gz']
    sdk=dict(source_revision=revision,binary_build_revision=compiled,binary_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest(),artifacts={f:hashlib.sha256((REPO/f).read_bytes()).hexdigest() for f in artifacts})
    for version in args.version or ['1.16.1','1.21.11']:
        run(version,args.binary.resolve(),args.jars.resolve(),check=functools.partial(check,binary=args.binary.resolve(),sdk=sdk))
if __name__=='__main__':main()
