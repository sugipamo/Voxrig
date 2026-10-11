#!/usr/bin/env python3
"""Qualify explicit local pair models seeded by original official spawn receipts.

These are not autonomous collision checks or server execution acknowledgements.
Caller predicates and velocity seeds remain explicit simulation inputs.
"""
import argparse
import copy
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
    report['sdk'] = sdk
    peer_pose_check(version, command, request, trace, report, binary, sdk,
                    disconnect_observer=False)
    native = json.loads(gzip.decompress((REPO/'data/client_api/entity_push_oracle.json.gz').read_bytes()))['versions'][version]['results']
    original = lambda scenario: next(r for r in native if r['first_kind']==1 and r['second_kind']==2 and r['scenario']==scenario)
    def entities():
        return request('entities')['entities']
    command('summon minecraft:cow 0.5 65 5.5 {Tags:["PushFirst"],NoAI:1b,NoGravity:1b,Invulnerable:1b}')
    command('summon minecraft:pig 4.5 65 5.5 {Tags:["PushSecond"],NoAI:1b,NoGravity:1b,Invulnerable:1b}')
    def pair():
        rows=entities()
        cow=next((e for e in rows if e['motion']['entity']['type_name']=='minecraft:cow'),None)
        pig=next((e for e in rows if e['motion']['entity']['type_name']=='minecraft:pig'),None)
        return (cow,pig) if cow and pig else None
    cow,pig=until(pair,10)
    proofs=[]
    for entity in (cow,pig):
        spawn=entity['motion']['entity'];source=spawn['spawn_position']['source']
        assert source['kind']=='received'
        received=[p for p in trace.since(0) if p['direction']=='clientbound' and p['phase'] in ('configuration','play')]
        frame=received[source['sequence']-1]
        assert frame['packet_id']==(0x02 if version=='1.16.1' else 0x01),frame
        r=NativeSocialReader(frame,version)
        assert r.integer()==spawn['id']['native_id']
        r.take(16)
        assert r.integer()==spawn['native_type_id']
        position=list(struct.unpack('>ddd',r.take(24)))
        assert position==spawn['spawn_position']['value']
        proofs.append(dict(identity=spawn['id'],position=spawn['spawn_position'],original_spawn=frame))
    report['checks'].append(dict(name='original_spawn_seed_receipts',proofs=proofs))
    # The model explicitly admits a call even at separation four. It does not
    # discover a contact from these historical spawn seeds or create live I/O.
    def body(entity, velocity):
        return dict(position=entity['motion']['entity']['spawn_position']['value'],
                    velocity=velocity,no_physics=False,vehicle=False,pushable=True)
    seed=dict(first=body(cow,[.125,-0.,.25]),second=body(pig,[-.5,.75,-.125]),
              first_sleeping=False,same_vehicle=False,legacy_pushthrough=0.)
    def evaluate(name,input,scenario):
        result=request('living_pair_push',input=input)
        assert 'Ok' in result['model'],result
        model=result['model']['Ok'];reference=original(scenario)
        assert model['input']==input
        assert result['velocity_bits']==[reference['first_velocity_bits'],reference['second_velocity_bits']],(result,reference)
        assert model['first_pushed']==reference['first_pushed'] and model['second_pushed']==reference['second_pushed']
        report['checks'].append(dict(name=name,explicit_input=input,result=result,
                                    original_case=dict(first_kind=1,second_kind=2,scenario=scenario)))
        return result
    saved=evaluate('explicit_admitted_pair_at_spawn_seeds',seed,8)
    assert evaluate('same_input_is_a_new_pure_model_call',seed,8)==saved
    reverse=copy.deepcopy(seed)
    reverse['first']['position'],reverse['second']['position']=reverse['second']['position'],reverse['first']['position']
    evaluate('reversed_model_receiver',reverse,9)
    sleeping=copy.deepcopy(seed);sleeping['first_sleeping']=True
    evaluate('explicit_sleeping_receiver',sleeping,15)
    riding=copy.deepcopy(seed);riding['same_vehicle']=True
    evaluate('explicit_shared_vehicle_predicate',riding,22)
    for index,scenario in [('first',17),('second',18)]:
        disabled=copy.deepcopy(seed);disabled[index]['no_physics']=True
        evaluate('explicit_'+index+'_no_physics',disabled,scenario)
    # Applying the result to a second model step adds another impulse. Receipt
    # deduplication/pair scheduling must belong to the caller, not this function.
    advanced=copy.deepcopy(seed)
    advanced['first']['velocity']=saved['model']['Ok']['first_velocity']
    advanced['second']['velocity']=saved['model']['Ok']['second_velocity']
    again=request('living_pair_push',input=advanced)
    assert 'Ok' in again['model'] and again['velocity_bits']!=saved['velocity_bits']
    report['checks'].append(dict(name='explicit_next_model_step',result=again))
    if version=='1.21.11':
        legacy=copy.deepcopy(seed);legacy['legacy_pushthrough']=.25
        result=request('living_pair_push',input=legacy)
        assert result==dict(model={'Err':'UnsupportedPushthrough'},velocity_bits=None)
        report['checks'].append(dict(name='modern_legacy_field_refused',result=result))
    # A historical numerical model remains a numerical model after removal;
    # it contains no EntityId or capability to address a replacement entity.
    identities=[e['motion']['entity']['id'] for e in (cow,pig)]
    command('kill @e[tag=PushFirst]');command('kill @e[tag=PushSecond]')
    until(lambda: all(e['motion']['entity']['id'] not in identities for e in entities()),10)
    assert request('living_pair_push',input=seed)==saved
    report['checks'].append(dict(name='retired_spawn_seed_has_no_live_target_authority',retired=identities))
    trace.expect_disconnect();request('disconnect')


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--accept-eula',action='store_true',required=True)
    p.add_argument('--binary',type=Path,required=True)
    p.add_argument('--jars',type=Path,default=REPO/'.local/climbing/downloads')
    p.add_argument('--version',choices=['1.16.1','1.21.11'],action='append')
    p.add_argument('--compiled-sdk-revision',required=True)
    args=p.parse_args()
    revision=subprocess.check_output(['git','rev-parse','HEAD'],text=True,cwd=REPO).strip()
    compiled=subprocess.check_output(['git','rev-parse',args.compiled_sdk_revision],text=True,cwd=REPO).strip()
    paths=['src','examples','Cargo.toml','Cargo.lock','data']
    assert not subprocess.check_output(['git','diff',compiled,'HEAD','--',*paths],cwd=REPO)
    assert not subprocess.check_output(['git','diff','HEAD','--',*paths],cwd=REPO)
    artifacts=['Cargo.lock','scripts/run_entity_push_live.py','scripts/run_entity_push_oracle.py',
               'scripts/run_entity_pose_live.py','scripts/run_climbing_control.py','scripts/run_common_native.py',
               'examples/climbing_control_probe.rs','data/client_api/entity_push_oracle.json.gz']
    sdk=dict(source_revision=revision,binary_build_revision=compiled,
             binary_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest(),
             artifacts={f:hashlib.sha256((REPO/f).read_bytes()).hexdigest() for f in artifacts})
    for version in args.version or ['1.16.1','1.21.11']:
        run(version,args.binary.resolve(),args.jars.resolve(),
            check=functools.partial(check,binary=args.binary.resolve(),sdk=sdk))
if __name__=='__main__':main()
