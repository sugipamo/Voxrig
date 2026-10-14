#!/usr/bin/env python3
"""Verify spatial model discovery with official spawn/motion/lifetime receipts.

Models are explicitly seeded and advanced here. Native query order is qualified
by the independent original oracle; packet-to-tick and owner integration belong
to separate work. No autonomous contact or server execution ACK is claimed.
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
    peer_pose_check(version, command, request, trace, report, binary, sdk, disconnect_observer=False)
    native = json.loads(gzip.decompress((REPO/'data/client_api/entity_query_oracle.json.gz').read_bytes()))['versions'][version]['results']
    for row in [r for r in native if r['seed'] in (0, 1, 4, 9, 15, 23)]:
        assert request('contact_index_new') == {'created': True}
        for chunk in row['known_chunks']:
            assert request('contact_index_chunk', chunk=chunk, known=True) == {'Ok': None}
        for step in row['steps']:
            for action in step['actions']:
                args = {k:v for k,v in action.items() if k != 'op'}
                result = request('contact_index_'+action['op'], **args)
                assert result == {'Ok': True if action['op']=='remove' else None}, result
            found = request('contact_index_query', client_side=row['client_side'], querying=row['querying'])
            assert found['Ok']['keys'] == step['keys'], (row['seed'], found, step)
        report['checks'].append(dict(name='original_query_schedule_'+str(row['seed'])+'_'+str(row['client_side']), queries=len(row['steps'])))

    command('difficulty normal')
    expected_positions={'cow':[.5,65.,5.5], 'pig':[-.5,65.,5.5], 'zombie':[.5,65.,-.5]}
    def entities(): return request('entities')['entities']
    def entity(kind): return next((e for e in entities() if e['motion']['entity']['type_name']=='minecraft:'+kind), None)
    def actual(sequence):
        packets=[p for p in trace.since(0) if p['direction']=='clientbound' and p['phase'] in ('configuration','play')]
        return packets[sequence-1]
    def geometry(kind):
        return next((e for e in request('entity_geometry') if e['entity']['type_name']=='minecraft:'+kind), None)
    models={}
    identities={}
    def seeded(kind):
        e=entity(kind);g=geometry(kind)
        if not e or not g or 'Ok' not in g['geometry']:return None
        shape=g['geometry']['Ok'];position=e['motion']['position']['value']['position']
        if version=='1.21.11' and shape['evidence']['scale']['attribute']['value']['base']!=1.:return None
        assert shape['entity']==e['motion']['entity']['id']
        assert shape['evidence']['position_source']==e['motion']['position']['source']
        spawn=e['motion']['entity'];key=spawn['id']['spawn_sequence']
        # Explicit numerical admission at a historical target, not a native tick.
        model=dict(key=key,kind=kind.capitalize(),body=dict(
            push=dict(position=position,velocity=[0.,0.,0.],no_physics=False,vehicle=False,pushable=True),
            bounds=shape['bounds'],sleeping=False,legacy_pushthrough=0.,team=None),
            spectator=False,player='NonPlayer',team_key=None,vehicle_root=None)
        return e,g,model

    for kind,pos in expected_positions.items():
        tag='Query'+kind.capitalize()
        command(f'summon minecraft:{kind} {pos[0]} {pos[1]} {pos[2]} {{Tags:["{tag}"],NoAI:1b,NoGravity:1b,Invulnerable:1b,PersistenceRequired:1b}}')
        e=until(lambda:entity(kind),10)
        if version=='1.21.11':
            command(f'attribute @e[tag={tag},limit=1] minecraft:scale base set 0.5')
            command(f'attribute @e[tag={tag},limit=1] minecraft:scale base set 1')
        e,g,model=until(lambda:seeded(kind),10)
        spawn=e['motion']['entity'];frame=actual(spawn['spawn_position']['source']['sequence'])
        assert frame['packet_id']==(0x02 if version=='1.16.1' else 0x01)
        r=NativeSocialReader(frame,version);assert r.integer()==spawn['id']['native_id'];r.take(16);assert r.integer()==spawn['native_type_id']
        assert list(struct.unpack('>ddd',r.take(24)))==spawn['spawn_position']['value']==pos
        models[kind]=model;identities[kind]=spawn['id']
        report['checks'].append(dict(name='original_'+kind+'_spawn_geometry_seed',receipt=e,geometry=g,original_packet=frame,explicit_model=model))

    assert request('contact_index_new')=={'created':True}
    for x in range(-1,2):
        for z in range(-1,2):assert request('contact_index_chunk',chunk=[x,z],known=True)=={'Ok':None}
    for model in models.values():assert request('contact_index_upsert',entity=model)=={'Ok':None}
    q=copy.deepcopy(models['cow']);q['key']=2**32;q['body']['bounds']=dict(min_x=-2.,min_y=64.,min_z=-2.,max_x=2.,max_y=69.,max_z=7.)
    def query():return request('contact_index_query',client_side=False,querying=q)
    order=['pig','zombie','cow'] if version=='1.16.1' else ['pig','cow','zombie']
    initial=query();assert initial['Ok']['keys']==[models[k]['key'] for k in order],initial
    assert request('contact_index_query',client_side=True,querying=q)['Ok']['keys']==[]
    report['checks'].append(dict(name='explicit_received_seed_index_query_order',result=initial,querying=q))
    saved=copy.deepcopy(initial)
    assert request('contact_index_chunk',chunk=[0,0],known=False)=={'Ok':None}
    assert query()=={'Err':'UnknownChunk'}
    assert request('contact_index_chunk',chunk=[0,0],known=True)=={'Ok':None}
    assert query()==saved
    report['checks'].append(dict(name='missing_membership_refuses_without_partial_candidates'))
    unsupported=copy.deepcopy(models['cow']);unsupported['key']=2**32+1;unsupported['kind']=None
    assert request('contact_index_upsert',entity=unsupported)=={'Ok':None}
    assert query()=={'Err':'UnsupportedBody'}
    assert request('contact_index_remove',key=unsupported['key'])=={'Ok':True}
    assert query()==saved
    report['checks'].append(dict(name='unqualified_intersecting_body_refuses_whole_query'))
    bad=copy.deepcopy(models['cow']);bad['body']['bounds']['min_x']=100.
    assert request('contact_index_upsert',entity=bad)=={'Err':{'InvalidBody':'InvalidBounds'}}
    assert query()==saved
    report['checks'].append(dict(name='failed_update_preserves_existing_index'))

    request('entity_history',tail=True)
    previous=entity('cow')['motion']['position']['source']['sequence']
    for i,pos in enumerate([[16.5,70.,5.5],[-.5,65.,6.5]]):
        command(f'tp @e[tag=QueryCow,limit=1] {pos[0]} {pos[1]} {pos[2]}')
        def moved():
            e=entity('cow')
            return e if e and e['motion']['position']['source']['sequence']>previous and e['motion']['position']['value']['position']==pos else None
        e=until(moved,10);source=e['motion']['position']['source'];frame=actual(source['sequence']);r=NativeSocialReader(frame,version)
        assert r.integer()==identities['cow']['native_id']
        assert frame['packet_id'] in ((0x56,) if version=='1.16.1' else (0x23,0x7b))
        assert list(struct.unpack('>ddd',r.take(24)))==pos
        page=request('entity_history');assert page['gap'] is None and not page['has_more']
        records=[v for v in page['records'] if v['receive_sequence']==source['sequence'] and 'Motion' in v['kind']]
        assert len(records)==1 and records[0]['kind']['Motion']['entity']['id']==identities['cow']
        e,g,model=until(lambda:(s if (s:=seeded('cow')) and s[2]['body']['push']['position']==pos else None),10)
        assert model['key']==models['cow']['key'];models['cow']=model
        assert request('contact_index_upsert',entity=model)=={'Ok':None}
        found=query();expected=[models['pig']['key'],models['zombie']['key']] if i==0 else [models['pig']['key'],models['cow']['key'],models['zombie']['key']]
        assert found['Ok']['keys']==expected,(version,found,expected)
        assert entity('cow')['motion']['position']==e['motion']['position']
        report['checks'].append(dict(name='original_motion_then_explicit_section_update_'+str(i),receipt=e,history=records[0],original_packet=frame,result=found))
        previous=source['sequence']

    old=identities['cow'];command('kill @e[tag=QueryCow]');until(lambda:not entity('cow'),10)
    assert request('contact_index_remove',key=models['cow']['key'])=={'Ok':True}
    assert query()['Ok']['keys']==[models['pig']['key'],models['zombie']['key']]
    command('summon minecraft:cow -0.5 65 6.5 {Tags:["QueryCow"],NoAI:1b,NoGravity:1b,Invulnerable:1b}')
    until(lambda:entity('cow'),10)
    if version=='1.21.11':
        command('attribute @e[tag=QueryCow,limit=1] minecraft:scale base set 0.5');command('attribute @e[tag=QueryCow,limit=1] minecraft:scale base set 1')
    e,g,replacement=until(lambda:seeded('cow'),10)
    assert e['motion']['entity']['id']!=old and replacement['key']!=models['cow']['key']
    assert request('contact_index_upsert',entity=replacement)=={'Ok':None}
    assert query()['Ok']['keys']==[models['pig']['key'],replacement['key'],models['zombie']['key']]
    assert request('contact_index_remove',key=models['cow']['key'])=={'Ok':False}
    report['checks'].append(dict(name='original_removal_respawn_and_distinct_model_lifetime',retired=old,replacement=e['motion']['entity']['id']))
    assert request('contact_index_invalidate')=={'invalidated':True}
    assert query()=={'Err':'Invalidated'}
    assert request('contact_index_upsert',entity=replacement)=={'Err':'Invalidated'}
    report['checks'].append(dict(name='explicit_lifetime_gap_irreversibly_invalidates_index'))
    trace.expect_disconnect();request('revoke_entity_history')


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--accept-eula',action='store_true',required=True)
    p.add_argument('--binary',type=Path,required=True)
    p.add_argument('--jars',type=Path,default=REPO/'.local/climbing/downloads')
    p.add_argument('--version',choices=['1.16.1','1.21.11'],action='append')
    p.add_argument('--compiled-sdk-revision',required=True)
    args=p.parse_args()
    revision=subprocess.check_output(['git','rev-parse','HEAD'],cwd=REPO,text=True).strip()
    compiled=subprocess.check_output(['git','rev-parse',args.compiled_sdk_revision],cwd=REPO,text=True).strip()
    paths=['src','examples','Cargo.toml','Cargo.lock','data']
    assert not subprocess.check_output(['git','diff',compiled,'HEAD','--',*paths],cwd=REPO)
    assert not subprocess.check_output(['git','diff','HEAD','--',*paths],cwd=REPO)
    artifacts=['Cargo.lock','scripts/run_entity_query_live.py','scripts/run_entity_query_oracle.py','scripts/entity_query_oracle/1.16.1/EntityQueryOracle.java','scripts/entity_query_oracle/1.21.11/EntityQueryOracle.java','scripts/run_entity_pose_live.py','scripts/run_climbing_control.py','scripts/run_common_native.py','examples/climbing_control_probe.rs','data/client_api/entity_query_oracle.json.gz']
    sdk=dict(source_revision=revision,binary_build_revision=compiled,binary_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest(),artifacts={f:hashlib.sha256((REPO/f).read_bytes()).hexdigest() for f in artifacts})
    for version in args.version or ['1.16.1','1.21.11']:
        run(version,args.binary.resolve(),args.jars.resolve(),check=functools.partial(check,binary=args.binary.resolve(),sdk=sdk))


if __name__=='__main__':main()
