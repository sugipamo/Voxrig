#!/usr/bin/env python3
"""Verify original team receipts and explicit ordered contact models while connected.

Numerical query poses/order/predicates come from the isolated native oracle;
received team rules/membership are verified against actual original frames.
No autonomous native ClientLevel tick or server contact execution is claimed.
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
import uuid
from run_climbing_control import REPO, run
from run_common_native import NativeSocialReader, decode_social_frame, until
from run_entity_pose_live import check as peer_pose_check

RULES=['always','never','push_other_teams','push_own_team']
WIRE=['always','never','pushOtherTeams','pushOwnTeam']


def check(version,command,request,trace,report,binary,sdk):
    report['sdk']=sdk
    peer_pose_check(version,command,request,trace,report,binary,sdk,disconnect_observer=False)
    rows=json.loads(gzip.decompress((REPO/'data/client_api/entity_contact_oracle.json.gz').read_bytes()))['versions'][version]['results']
    def entities():return request('entities')['entities']
    command('summon minecraft:cow 0.5 65 5.5 {Tags:["ContactCow"],NoAI:1b,NoGravity:1b,Invulnerable:1b,Age:0}')
    cow=until(lambda: next((e for e in entities() if e['motion']['entity']['type_name']=='minecraft:cow'),None),10)
    spawn=cow['motion']['entity'];holder=str(uuid.UUID(bytes=bytes(spawn['uuid'])))
    def frame(sequence):
        received=[p for p in trace.since(0) if p['direction']=='clientbound' and p['phase'] in ('configuration','play')]
        return received[sequence-1]
    original=frame(spawn['spawn_position']['source']['sequence'])
    assert original['packet_id']==(2 if version=='1.16.1' else 1)
    r=NativeSocialReader(original,version);assert r.integer()==spawn['id']['native_id'];r.take(16);assert r.integer()==spawn['native_type_id']
    assert list(struct.unpack('>ddd',r.take(24)))==spawn['spawn_position']['value']
    report['checks'].append(dict(name='original_cow_lifetime_seed',spawn=spawn,original_spawn=original))
    command('team add contactq');command('team add contactc')
    def evaluate(row):
        result=request('living_contacts',client_side=row['client_side'],querying=row['querying'],candidates=row['candidates'])
        assert 'Ok' in result['model'],result
        model=result['model']['Ok'];expected=row['results']
        assert result['velocity_bits']['querying']==expected[-1]['querying_velocity_bits'],(result,row)
        for i,c in enumerate(model['candidates']):
            assert c['candidate_index']==i
            assert (c['disposition']=='Admitted')==expected[i]['admitted'],(result,row)
            if c['push']:
                assert result['velocity_bits']['candidates'][i]==expected[i]['candidate_velocity_bits'],(result,row)
                assert c['push']['first_pushed']==expected[i]['candidate_pushed']
                assert c['push']['second_pushed']==expected[i]['querying_pushed']
        return result
    def original_case(qr,cr,allied,scenario=0,kind=0,client=True):
        return next(r for r in rows if (r['query_rule'],r['candidate_rule'],r['allied'],r['scenario'],r['kind'],r['client_side'])==(qr,cr,allied,scenario,kind,client))
    for index,(qr,cr,allied) in enumerate([(q,c,False)for q in range(-1,4)for c in range(-1,4)]+[(r,r,True)for r in range(4)]):
        command('team leave @e[tag=ContactCow]');command('team leave ClimbingProbe')
        if qr>=0:
            command(f'team modify contactq collisionRule {WIRE[qr]}');command('team join contactq @e[tag=ContactCow]')
        if cr>=0:
            if not allied:command(f'team modify contactc collisionRule {WIRE[cr]}')
            command('team join '+('contactq' if allied else 'contactc')+' ClimbingProbe')
        def ready():
            capture=request('teams');teams={t['name']:t for t in capture['teams']}
            def membership(h):return next((t for t in teams.values() if any(m['value']==h for m in t['members'])),None)
            q=membership(holder);c=membership('ClimbingProbe')
            if (q is None)!=(qr<0) or (c is None)!=(cr<0):return None
            if q and q['parameters']['value']['collision']!=RULES[qr]:return None
            if c and c['parameters']['value']['collision']!=RULES[cr]:return None
            if bool(q and c and q['name']==c['name'])!=allied:return None
            return capture,q,c
        capture,q,c=until(ready,10)
        proofs=[]
        for team in [q,c]:
            if not team:continue
            receipt=team['parameters'];actual=frame(receipt['source']['sequence']);packet=decode_social_frame(version,actual)
            assert packet['name']==team['name'] and packet['parameters']==receipt['value']
            proofs.append(actual)
            for member in team['members']:
                actual=frame(member['source']['sequence']);packet=decode_social_frame(version,actual)
                assert packet['name']==team['name'] and packet['operation'] in (0,3) and member['value'] in packet['members']
                proofs.append(actual)
        row=copy.deepcopy(original_case(qr,cr,allied))
        row['querying']['team']=q['parameters']['value']['collision'] if q else None
        row['candidates'][0]['body']['team']=c['parameters']['value']['collision'] if c else None
        row['candidates'][0]['allied']=bool(q and c and q['name']==c['name'])
        result=evaluate(row)
        report['checks'].append(dict(name='original_team_contact_rules_'+str(index),received_teams=capture,original_frames=proofs,native_case={k:row[k]for k in ('query_rule','candidate_rule','allied','scenario','kind','client_side')},numerical_model=result))
    command('team remove contactq');command('team remove contactc')
    removed=until(lambda: (v if not v['teams'] else None) if (v:=request('teams')) else None,10)
    removal=frame(removed['last_update_sequence']);assert decode_social_frame(version,removal)['operation']==1
    report['checks'].append(dict(name='original_team_removal',received_teams=removed,original_frame=removal))
    for scenario in range(1,22):
        row=original_case(-1,-1,False,scenario)
        report['checks'].append(dict(name='original_numerical_contact_case_'+str(scenario),numerical_model=evaluate(row)))
    for kind in [1,2]:
        row=original_case(-1,-1,False,kind=kind)
        result=evaluate(row);assert result['model']['Ok']['candidates'][0]['disposition']=='NonLocalPlayer'
        report['checks'].append(dict(name='client_candidate_class_'+str(kind),numerical_model=result))
    for scenario in [0,20]:
        row=original_case(-1,-1,False,scenario,client=False)
        result=evaluate(row);assert all(c['push'] for c in result['model']['Ok']['candidates'])
        report['checks'].append(dict(name='explicit_server_order_'+str(scenario),numerical_model=result))
    row=original_case(-1,-1,False);saved=evaluate(row)
    for label,candidates,error in [('budget',row['candidates']*17,'TooManyCandidates'),('late_bounds',copy.deepcopy(row['candidates']),'InvalidBounds'),('teams',copy.deepcopy(row['candidates']),'InconsistentTeams')]:
        if label=='late_bounds':
            candidates[-1]['body']['bounds']['min_x']=2.
            candidates[-1]['body'].pop('bounds_bits',None)
        if label=='teams':candidates[-1]['allied']=True
        result=request('living_contacts',client_side=True,querying=row['querying'],candidates=candidates)
        assert result=={'model':{'Err':error},'velocity_bits':None},result
        assert evaluate(row)==saved
        report['checks'].append(dict(name='whole_batch_refusal_'+label,result=result))
    command('kill @e[tag=ContactCow]')
    until(lambda: all(e['motion']['entity']['id']!=spawn['id'] for e in entities()),10)
    command('summon minecraft:cow 0.5 65 5.5 {Tags:["ContactCow"],NoAI:1b,NoGravity:1b,Invulnerable:1b}')
    replacement=until(lambda: next((e for e in entities() if e['motion']['entity']['type_name']=='minecraft:cow'),None),10)['motion']['entity']
    assert replacement['id']!=spawn['id']
    assert evaluate(row)==saved  # Contains no live target; saved numerical inputs stay numerical.
    report['checks'].append(dict(name='removed_lifetime_and_replacement_leave_no_model_send_authority',retired=spawn['id'],replacement=replacement['id']))
    trace.expect_disconnect();request('disconnect')


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--accept-eula',action='store_true',required=True)
    p.add_argument('--binary',type=Path,required=True)
    p.add_argument('--jars',type=Path,default=REPO/'.local/climbing/downloads')
    p.add_argument('--version',choices=['1.16.1','1.21.11'],action='append')
    p.add_argument('--compiled-sdk-revision',required=True)
    args=p.parse_args();revision=subprocess.check_output(['git','rev-parse','HEAD'],text=True,cwd=REPO).strip();compiled=subprocess.check_output(['git','rev-parse',args.compiled_sdk_revision],text=True,cwd=REPO).strip()
    paths=['src','examples','Cargo.toml','Cargo.lock','data']
    assert not subprocess.check_output(['git','diff',compiled,'HEAD','--',*paths],cwd=REPO)
    assert not subprocess.check_output(['git','diff','HEAD','--',*paths],cwd=REPO)
    artifacts=['Cargo.lock','scripts/run_entity_contact_live.py','scripts/run_entity_contact_oracle.py','scripts/run_entity_pose_live.py','scripts/run_climbing_control.py','scripts/run_common_native.py','examples/climbing_control_probe.rs','data/client_api/entity_contact_oracle.json.gz']
    sdk=dict(source_revision=revision,binary_build_revision=compiled,binary_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest(),artifacts={f:hashlib.sha256((REPO/f).read_bytes()).hexdigest()for f in artifacts})
    for version in args.version or ['1.16.1','1.21.11']:
        run(version,args.binary.resolve(),args.jars.resolve(),check=functools.partial(check,binary=args.binary.resolve(),sdk=sdk))
if __name__=='__main__':main()
