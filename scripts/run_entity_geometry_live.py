#!/usr/bin/env python3
"""Qualify sampled pose/baby/scale geometry on unchanged official loopback servers."""
import argparse
import copy
import gzip
import functools
import hashlib
import json
from pathlib import Path
import struct
import subprocess
from run_climbing_control import REPO, run
from run_common_native import NativeSocialReader, PacketTraceProxy, until
from run_entity_pose_live import check as peer_pose_check


def check(version, command, request, trace, report, binary, sdk):
    report['sdk'] = sdk
    oracle = json.loads(gzip.decompress((REPO / 'data/client_api/entity_dimensions_oracle.json.gz').read_bytes()))['versions'][version]['results']
    def rows():
        return request('entity_geometry')
    def metadata(original):
        r=NativeSocialReader(original,version);target=r.integer();values={}
        while True:
            index=r.take(1)[0]
            if index==255:break
            kind=r.integer()
            old=version=='1.16.1'
            if kind==0:value=int.from_bytes(r.take(1),'big',signed=True)
            elif kind==1 or kind in ((11,13,18) if old else tuple(range(19,29))+(12,14,15,31,32,33,34,38)):value=r.integer()
            elif kind==(2 if old else 3):value=struct.unpack('>f',r.take(4))[0]
            elif kind==(7 if old else 8):value=r.boolean()
            elif kind in ((3,4) if old else (4,)):value=r.string()
            elif kind==(5 if old else 6):
                if r.boolean():r.text()
                value=None
            elif old and kind==14:
                tag=r.take(1)[0]
                if tag:
                    r.take(int.from_bytes(r.take(2),'big'));r.nbt(tag)
                value=None
            elif not old and kind==5:r.text();value=None
            elif not old and kind==17:
                for _ in range(r.integer()):
                    assert r.integer()==21  # Original entity_effect ARGB particle.
                    r.take(4)
                value=None
            elif kind==(10 if old else 11):
                if r.boolean():r.take(8)
                value=None
            else:raise AssertionError(('unqualified original metadata field',kind,index))
            values[index]=(kind,value)
        r.end();return target,values
    def actual(sequence, packet):
        frames = [p for p in trace.since(0) if p['direction'] == 'clientbound' and p['phase'] in ('configuration', 'play')]
        result = frames[sequence - 1]
        assert result['phase'] == 'play' and result['packet_id'] == packet, result
        return result
    def verify(row, *, requested=1., baby=False):
        assert 'Ok' in row['geometry'], row
        g = row['geometry']['Ok']; evidence = g['evidence']; pose = evidence['pose']['value']['Pose']
        native = next(r for r in oracle if r['type'] == row['entity']['type_name'] and r['pose'] == pose and r['baby'] == baby and r['requested_scale'] == requested)
        # Public floats are serialized as shortest f32 values; convert back before
        # comparing the exact native double expansion and box construction.
        f32 = lambda n: struct.unpack('>f', struct.pack('>f', n))[0]
        assert f32(g['width']) == native['width'] and f32(g['height']) == native['height'], (g,native)
        entity = request('entities')['entities']
        parent = next(e for e in entity if e['motion']['entity']['id'] == g['entity'])
        position = parent['motion']['position']
        assert evidence['position_source'] == position['source']
        assert g['position_error'] == position['value']['quantization_error']
        half = native['width'] / 2.; p = position['value']['position']
        expected = [p[0]-half,p[1],p[2]-half,p[0]+half,p[1]+native['height'],p[2]+half]
        box = g['bounds']; assert [box[k] for k in ('min_x','min_y','min_z','max_x','max_y','max_z')] == expected
        assert g['entity'] == row['entity']['id']
        proofs = []
        if evidence['pose']['source'] != 'TypeDefault':
            original = actual(evidence['pose']['source']['Received']['sequence'],0x44 if version=='1.16.1' else 0x61)
            target, fields = metadata(original)
            assert target == g['entity']['native_id']
            assert fields[6] == (18 if version=='1.16.1' else 20,pose)
            proofs.append(original)
        scale = evidence['scale']
        if version == '1.21.11':
            source = scale['attribute']['source']; assert source['kind'] == 'received'
            original = actual(source['sequence'],0x81)
            r=NativeSocialReader(original,version);target=r.integer();attributes={}
            for _ in range(r.integer()):
                key=r.integer();base=struct.unpack('>d',r.take(8))[0];modifiers=[]
                for _ in range(r.integer()):
                    identifier=r.string();amount=struct.unpack('>d',r.take(8))[0];operation=r.take(1)[0]
                    modifiers.append(dict(id=identifier,amount=amount,operation=('Addition','MultiplyBase','MultiplyTotal')[operation]))
                attributes[key]=dict(base=base,modifiers=modifiers)
            r.end()
            assert target == g['entity']['native_id']
            assert attributes[25] == scale['attribute']['value']
            assert f32(scale['value']) == native['effective_scale']
            proofs.append(original)
        else:
            assert scale == dict(value=1.,attribute=None)
        if evidence['baby'] and evidence['baby']['source'] != 'TypeDefault':
            original = actual(evidence['baby']['source']['Received']['sequence'],0x44 if version=='1.16.1' else 0x61)
            target, fields = metadata(original)
            assert target == g['entity']['native_id']
            assert fields[evidence['baby']['index']] == (7 if version=='1.16.1' else 8,baby)
            assert evidence['baby']['value'] == {'Bool':baby}
            proofs.append(original)
        return dict(observation=row, original_inputs=proofs, original_dimension_case=dict(type=native['type'],pose=pose,baby=baby,requested_scale=requested))
    # Reuse the unchanged two-client spawn/crouch/stand/removal check. Wrap the
    # observer's pose reads to verify geometry at each actual pose transition.
    def with_geometry(action, **args):
        result = request(action, **args)
        if action == 'entity_poses' and len(result)==1 and result[0]['pose']:
            peer = result[0]
            if peer['pose']['pose'] in ('crouching','standing'):
                observed = next((r for r in rows() if r['entity']['id']==peer['entity']['id']),None)
                if observed and 'Ok' in observed['geometry']:
                    pose = observed['geometry']['Ok']['evidence']['pose']
                    if pose['value']=={'Pose':peer['pose']['native_id']}:
                        key='peer_'+peer['pose']['pose']
                        if not any(c['name']==key for c in report['checks']):
                            report['checks'].append(dict(name=key,**verify(observed)))
        return result
    peer_pose_check(version,command,with_geometry,trace,report,binary,sdk,disconnect_observer=False)
    assert all(any(c['name']==n for c in report['checks']) for n in ('peer_crouching','peer_standing'))
    command('difficulty normal')  # Original peaceful servers immediately remove zombies.
    for kind in ('cow','pig','zombie'):
        tag='GeometryMob'
        command('kill @e[tag='+tag+']')
        body='IsBaby:1b' if kind=='zombie' else 'Age:-24000'
        command(f'summon minecraft:{kind} 2.5 65 2.5 {{Tags:["{tag}"],NoAI:1b,NoGravity:1b,Invulnerable:1b,PersistenceRequired:1b,{body}}}')
        def current():
            return next((r for r in rows() if r['entity']['type_name']=='minecraft:'+kind),None)
        def baby_ready():
            row=current()
            return row if row and 'Ok' in row['geometry'] and row['geometry']['Ok']['evidence']['baby']['value']=={'Bool':True} else None
        row=until(baby_ready,10)
        report['checks'].append(dict(name=kind+'_baby_dimensions',**verify(row,baby=True)))
        identity=copy.deepcopy(row['entity']['id'])
        if version=='1.21.11':
            for amount in (.5,1.37,2.,32.,1.):
                before=current()['geometry']['Ok']['evidence']['scale']['attribute']['source']['sequence']
                command(f'attribute @e[tag={tag},limit=1] minecraft:scale base set {amount}')
                def scaled():
                    row=current()
                    if not row or 'Ok' not in row['geometry']:return None
                    attr=row['geometry']['Ok']['evidence']['scale']['attribute']
                    return row if attr['source']['sequence']>before and attr['value']['base']==amount else None
                row=until(scaled,5)
                assert row['entity']['id']==identity
                report['checks'].append(dict(name=kind+'_scale_'+str(amount),**verify(row,baby=True,requested=amount)))
        if version=='1.21.11':
            modifiers=[('voxrig:geometry_add',.25,'add_value'),('voxrig:geometry_base',.2,'add_multiplied_base'),('voxrig:geometry_total',1/3,'add_multiplied_total')]
            for identifier,amount,operation in modifiers:
                command(f'attribute @e[tag={tag},limit=1] minecraft:scale modifier add {identifier} {amount} {operation}')
            def modified():
                row=current()
                if not row or 'Ok' not in row['geometry']:return None
                attribute=row['geometry']['Ok']['evidence']['scale']['attribute']['value']
                return row if len(attribute['modifiers'])==3 else None
            row=until(modified,5)
            assert row['geometry']['Ok']['evidence']['scale']['value']==2.
            native_value=command(f'attribute @e[tag={tag},limit=1] minecraft:scale get 1000000')
            assert '2000000' in native_value,native_value
            report['checks'].append(dict(name=kind+'_original_three_modifier_operations',native_attribute_value=native_value,**verify(row,baby=True,requested=2.)))
            for identifier,_,_ in modifiers:
                command(f'attribute @e[tag={tag},limit=1] minecraft:scale modifier remove {identifier}')
            until(lambda: (row if (row:=current()) and 'Ok' in row['geometry'] and not row['geometry']['Ok']['evidence']['scale']['attribute']['value']['modifiers'] else None),5)
        if kind=='zombie':
            # Legacy original load only calls setBaby for true IsBaby. A false
            # NBT merge is not an adult transition or an acknowledgement.
            command('kill @e[tag='+tag+']')
            until(lambda: not any(r['entity']['id']==identity for r in rows()),5)
            report['checks'].append(dict(name='zombie_baby_lifetime_retired',identity=identity))
            command(f'summon minecraft:zombie 2.5 65 2.5 {{Tags:["{tag}"],NoAI:1b,NoGravity:1b,Invulnerable:1b,PersistenceRequired:1b,IsBaby:0b}}')
        else:
            command(f'data merge entity @e[tag={tag},limit=1] '+'{Age:0}')
        def adult_ready():
            row=current()
            return row if row and 'Ok' in row['geometry'] and row['geometry']['Ok']['evidence']['baby']['value']=={'Bool':False} else None
        row=until(adult_ready,5)
        if kind=='zombie':
            assert row['entity']['id']!=identity
            identity=copy.deepcopy(row['entity']['id'])
        else:
            assert row['entity']['id']==identity
        report['checks'].append(dict(name=kind+('_new_spawn_adult_dimensions' if kind=='zombie' else '_adult_dimensions'),**verify(row)))
        saved=copy.deepcopy(row)
        command('kill @e[tag='+tag+']')
        until(lambda: not any(r['entity']['id']==identity for r in rows()),5)
        assert saved==row
        report['checks'].append(dict(name=kind+'_despawn_retires_geometry',identity=identity))
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
    artifacts=['Cargo.lock','scripts/run_entity_geometry_live.py','scripts/run_entity_pose_live.py','scripts/run_climbing_control.py','scripts/run_common_native.py','examples/climbing_control_probe.rs','data/client_api/entity_dimensions_oracle.json.gz']
    sdk=dict(source_revision=revision,binary_build_revision=compiled,binary_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest(),artifacts={f:hashlib.sha256((REPO/f).read_bytes()).hexdigest() for f in artifacts})
    for version in args.version or ['1.16.1','1.21.11']:
        run(version,args.binary.resolve(),args.jars.resolve(),check=functools.partial(check,binary=args.binary.resolve(),sdk=sdk))
if __name__=='__main__':main()
