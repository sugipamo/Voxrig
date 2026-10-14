#!/usr/bin/env python3
"""Received equipment movement inputs on unchanged official loopback servers."""
import argparse
import functools
import hashlib
from pathlib import Path
import re
import struct
import subprocess
import uuid
from run_climbing_control import REPO, run
from run_common_native import NativeSocialReader, until


def original_attribute(version, player, name, trace):
    receipt = player['attributes'][name]
    peers = [p for p in trace.since(0) if p['direction']=='clientbound' and p['phase'] in ('configuration','play')]
    source = receipt['source']
    if source['kind']!='received': raise RuntimeError('attribute has no original receipt')
    packet = peers[source['sequence']-1]
    if packet['phase']!='play' or packet['packet_id']!=(0x58 if version=='1.16.1' else 0x81):
        raise RuntimeError('attribute ordinal points to another native packet')
    r = NativeSocialReader(packet, version)
    if r.integer()!=player['entity_id']: raise RuntimeError('attribute belongs to another entity')
    count = int.from_bytes(r.take(4),'big',signed=True) if r.old else r.integer()
    candidates = []
    for _ in range(count):
        key = r.string() if r.old else r.integer()
        base = struct.unpack('>d',r.take(8))[0]
        modifiers=[]
        for _ in range(r.integer()):
            key_id = str(uuid.UUID(bytes=r.take(16))) if r.old else r.string()
            amount=struct.unpack('>d',r.take(8))[0]
            op=r.take(1)[0]
            modifiers.append(dict(id=key_id,amount=amount,operation=('Addition','MultiplyBase','MultiplyTotal')[op]))
        candidates.append(dict(key=key,base=base,modifiers=modifiers))
    r.end()
    value=receipt['value']
    if not any(c['base']==value['base'] and c['modifiers']==value['modifiers'] for c in candidates):
        raise RuntimeError('attribute fields differ from original native receipt')
    return dict(receipt=receipt,original=packet)


def check(version, command, request, trace, report, *, sdk):
    report['sdk']=sdk
    old=version=='1.16.1'
    def replace(stack):
        command(('replaceitem entity ClimbingProbe armor.feet minecraft:' if old else 'item replace entity ClimbingProbe armor.feet with minecraft:')+stack)
    def received_boots(before, empty=False):
        p=request('player');s=p['inventory']['slots'][8]
        return p if s and s['source']['kind']=='received' and s['source']['sequence']>before and s['value']['kind']==('empty' if empty else 'item') else None
    def keys(**changes):
        return request('keys',controls=dict(forward=changes.get('forward',0),strafe=0,jump=False,sneak=changes.get('sneak',False),sprint=changes.get('sprint',False),yaw=0.,pitch=0.))
    def capture(name, record, expected):
        model=record['movement_equipment']
        if not model['available'] or any(model[k]!=v for k,v in expected.items()):
            raise RuntimeError('actual received equipment did not reach movement model: '+str(model))
        if record['status']['status']!='running' or record['corrections']:
            raise RuntimeError('equipment control paused/stopped/corrected')
        native=command('data get entity ClimbingProbe Pos')
        match=re.search(r'\[([^]]+)\]',native)
        position=[float(v.strip().rstrip('df')) for v in match[1].split(',')]
        if max(abs(a-b) for a,b in zip(position,record['frame']['position']))>.4:
            raise RuntimeError('native equipment endpoint differs')
        flags=int(record['frame']['on_ground'])
        if not old:flags|=int(record['frame']['horizontal_collision'])<<1
        payload=struct.pack('>dddff',*record['frame']['position'],record['controls']['yaw'],record['controls']['pitch'])+bytes([flags])
        wire=until(lambda:[p for p in trace.since(0) if p['direction']=='serverbound' and p['phase']=='play' and p['packet_id']==(0x13 if old else 0x1e) and p['body_hex']==payload.hex()],5)
        report['checks'].append(dict(name=name,record=record,player=request('player'),native_position=position,original_movement=wire[-1]))
    command('effect give ClimbingProbe minecraft:water_breathing 600 0 true')
    command('fill -8 63 -8 8 68 8 minecraft:air')
    command('fill -8 64 -8 8 64 8 minecraft:stone')
    command('fill -8 65 -8 8 67 8 minecraft:water[level=0]')
    command('tp ClimbingProbe 0.5 65 0.5 0 0')
    request('prepare',position=[.5,65.,.5])
    before=request('player')['receive_sequence']
    replace('diamond_boots{Enchantments:[{id:"minecraft:depth_strider",lvl:3s}]} 1' if old else 'diamond_boots[enchantments={"minecraft:depth_strider":3}] 1')
    player=until(lambda:received_boots(before),5)
    if not old:
        player=until(lambda:(p if (p:=request('player'))['attributes'].get('minecraft:water_movement_efficiency',{}).get('value',{}).get('value')==1. else None),5)
        report['checks'].append(dict(name='original_depth_strider_attribute',**original_attribute(version,player,'minecraft:water_movement_efficiency',trace)))
    request('start');keys(forward=1)
    expected={'depth_strider':3} if old else {'water_movement_efficiency':1.}
    capture('equipped_water_control',request('ticks',count=12),expected)
    before=request('player')['receive_sequence'];replace('air')
    until(lambda:received_boots(before,True),5)
    if not old:
        until(lambda:(p if (p:=request('player'))['attributes']['minecraft:water_movement_efficiency']['value']['value']==0. else None),5)
    capture('equipment_removed_during_same_owned_control',request('ticks',count=4),{'depth_strider':0,'water_movement_efficiency':0.})
    keys();stopped=request('stop')
    if request('record')!=stopped: raise RuntimeError('stopped equipment diagnostic changed')
    report['checks'].append(dict(name='stopped_equipment_diagnostic_retained',record=stopped))
    if not old:
        command('attribute ClimbingProbe minecraft:water_movement_efficiency base set 0.1')
        for key,amount,op in [('a',.3,'add_value'),('b',.5,'add_multiplied_base'),('c',.5,'add_multiplied_total')]:
            command(f'attribute ClimbingProbe minecraft:water_movement_efficiency modifier add voxrig:water_{key} {amount} {op}')
        expected=.9000000000000001
        player=until(lambda:(p if (p:=request('player'))['attributes']['minecraft:water_movement_efficiency']['value']['value']==expected else None),5)
        report['checks'].append(dict(name='original_three_operation_water_attribute',native=command('attribute ClimbingProbe minecraft:water_movement_efficiency get 1000000'),**original_attribute(version,player,'minecraft:water_movement_efficiency',trace)))
        request('start');keys(forward=1)
        capture('three_operation_water_attribute_reaches_model',request('ticks',count=8),{'water_movement_efficiency':expected})
        keys();request('stop')
    else:
        command('fill -8 65 -8 8 68 8 minecraft:air')
        command('fill -8 64 -8 8 64 8 minecraft:soul_sand')
        command('tp ClimbingProbe 0.5 64.875 0.5 0 0')
        request('prepare',position=[.5,64.875,.5])
        before=request('player')['receive_sequence'];replace('diamond_boots{Enchantments:[{id:"minecraft:soul_speed",lvl:1s}]} 1')
        until(lambda:received_boots(before),5)
        # Native onChangedBlock adds the server-owned speed modifier only
        # after moving onto a new supporting block, not while waiting idle.
        request('start');keys(forward=1)
        player=until(lambda:(p if (p:=request('player'))['attributes']['minecraft:movement_speed']['value']['value']>.13 else None),5)
        report['checks'].append(dict(name='original_server_owned_soul_speed_modifier',**original_attribute(version,player,'minecraft:movement_speed',trace)))
        record=request('ticks',count=12)
        capture('received_soul_speed_tag_reaches_model',record,{'soul_speed_blocks':['minecraft:soul_sand','minecraft:soul_soil']})
        keys();request('stop')
    # Both editions preserve local posture/momentum across explicit stop/start.
    restarted=request('restart')
    if restarted['first']['session_id']==restarted['stopped']['session_id']:raise RuntimeError('restart reused the previous owner')
    report['checks'].append(dict(name='equipment_control_explicit_restart',record=restarted))
    request('stop');trace.expect_disconnect();request('disconnect')


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--accept-eula',action='store_true',required=True)
    p.add_argument('--version',choices=('1.16.1','1.21.11'),action='append')
    p.add_argument('--binary',type=Path,required=True)
    p.add_argument('--jars',type=Path,default=REPO/'.local/climbing/downloads')
    p.add_argument('--compiled-sdk-revision',required=True)
    args=p.parse_args();revision=subprocess.check_output(['git','rev-parse','HEAD'],cwd=REPO,text=True).strip()
    compiled=subprocess.check_output(['git','rev-parse',args.compiled_sdk_revision],cwd=REPO,text=True).strip()
    if subprocess.check_output(['git','diff',compiled,'HEAD','--','src','examples','Cargo.toml','Cargo.lock'],cwd=REPO):raise RuntimeError('compiled SDK differs from runtime source')
    files=('Cargo.lock','scripts/run_equipment_movement.py','scripts/run_climbing_control.py','scripts/run_common_native.py','examples/climbing_control_probe.rs','scripts/movement_oracle/run.py','scripts/movement_oracle/equipment_scenarios.json','scripts/movement_oracle/java_1_16_1/MovementOracle.java','scripts/movement_oracle/java_1_21_11/MovementOracle.java','data/client_api/equipment_movement_oracle.json.gz')
    sdk=dict(source_revision=revision,binary_build_revision=compiled,binary_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest(),source_diff_sha256=hashlib.sha256(subprocess.check_output(['git','diff','HEAD'],cwd=REPO)).hexdigest(),artifacts={f:hashlib.sha256((REPO/f).read_bytes()).hexdigest() for f in files})
    for version in args.version or ('1.16.1','1.21.11'):
        run(version,args.binary.resolve(),args.jars.resolve(),check=functools.partial(check,sdk=sdk))
if __name__=='__main__':main()
