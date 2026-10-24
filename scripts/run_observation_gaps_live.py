#!/usr/bin/env python3
"""Observe dropped stacks, sparse-inventory digging, pose and palette properties on official servers.

The established runner uses a disposable loopback server, independent RCON and
an unchanged-byte protocol trace. No packets are fabricated. Requires --accept-eula.
"""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import threading
from run_climbing_control import REPO, run
from run_common_native import PacketTraceProxy, until


def check(version, command, request, trace, report):
    command('tp ClimbingProbe 0.5 65 0.5 0 0')
    request('prepare', position=[.5,65.,.5])
    region=dict(min=[0,65,0],max=[3,66,1])
    def observe():
        return request('observation_gaps',region=region)
    def block(pos):
        return next(b['state'] for b in observe()['capture']['world']['blocks'] if b['position']==pos)
    def receipt(value, entity):
        assert value['source']['kind']=='received',value
        seq=value['source']['sequence']
        packet=[p for p in trace.since(0) if p['direction']=='clientbound' and p['phase'] in ('configuration','play')][seq-1]
        assert packet['packet_id']==(0x44 if version=='1.16.1' else 0x61),packet
        native,_=PacketTraceProxy.varint(bytes.fromhex(packet['body_hex']))
        assert native==entity,(native,entity,packet)
        return packet

    # Native palette values must survive both block updates and common capture.
    for pos,state,properties in [
        ([1,65,0],'minecraft:oak_slab[type=bottom,waterlogged=true]',dict(type='bottom',waterlogged='true')),
        ([2,65,0],'minecraft:oak_stairs[facing=east,half=top,shape=straight,waterlogged=true]',dict(facing='east',half='top',shape='straight',waterlogged='true')),
        ([3,65,0],'minecraft:water[level=7]',dict(level='7'))]:
        command('setblock %d %d %d %s'%(*pos,state))
        observed=until(lambda: (b:=block(pos)) and b['properties']==properties and b,5)
        report['checks'].append(dict(name='received_palette_'+state,observed=observed,native=command('execute if block %d %d %d %s'%(*pos,state))))
    # Freeze water to a source before scheduled fluid ticks change its level.
    command('setblock 3 65 0 minecraft:water[level=0]')
    until(lambda: block([3,65,0])['properties'].get('level')=='0',5)
    report['checks'].append(dict(name='received_water_source',observation=observe()))

    # Item metadata contains lossless native custom data, not just an entity type.
    item=('{id:"minecraft:diamond",Count:3b,tag:{VoxrigProbe:42}}' if version=='1.16.1' else
          '{id:"minecraft:diamond",count:3,components:{"minecraft:custom_data":{VoxrigProbe:42}}}')
    command('summon minecraft:item 3.5 66 3.5 {Tags:["GapItem"],NoGravity:1b,PickupDelay:32767s,Item:'+item+'}')
    def dropped():
        return next((s for s in observe()['items']['stacks'] if s['stack']['value'].get('item',{}).get('name')=='minecraft:diamond'),None)
    stack=until(dropped,10)
    assert stack['index']==(7 if version=='1.16.1' else 8),stack
    assert stack['stack']['value']['item']['count']==3,stack
    data=stack['stack']['value']['item']['data']
    encoded=(data['bytes'] if version=='1.16.1' else next(c['bytes'] for c in data['patch']['added'] if c['definition']['name']=='minecraft:custom_data'))
    assert b'VoxrigProbe' in bytes(encoded) and bytes([0,0,0,42]) in bytes(encoded),data
    original=receipt(stack['stack'],stack['entity']['native_id'])
    native=command('data get entity @e[tag=GapItem,limit=1] Item')
    assert 'VoxrigProbe: 42' in native,native
    report['checks'].append(dict(name='received_dropped_item_id_count_data',observation=stack,packet=original,native=native))
    command('kill @e[tag=GapItem]')
    until(lambda: not observe()['items']['stacks'],10)

    # Retire the fluid fixture before the separate dry dig timing model.
    command('fill -8 65 -8 8 68 8 minecraft:air')
    command('tp ClimbingProbe 0.5 65 0.5 0 0')
    request('prepare',position=[.5,65.,.5])

    # Only establish the held hand through real server inventory changes.
    # Empty unrelated slots are never inferred locally.
    request('select_hotbar',slot=0)
    command('setblock 1 65 0 minecraft:dirt')
    until(lambda: block([1,65,0])['name']=='minecraft:dirt',5)
    before=observe()['capture']['player']
    missing=request('gap_estimate',target=[1,65,0])
    report['checks'].append(dict(name='initial_dig_inventory_knowledge',player=before,estimate=missing))
    command('give ClimbingProbe minecraft:stick')
    until(lambda: (v:=observe()['capture']['player']['inventory']['slots'][36]) and v['value']['kind']=='item',5)
    command('clear ClimbingProbe')
    until(lambda: (v:=observe()['capture']['player']['inventory']['slots'][36]) and v['value']['kind']=='empty',5)
    dug=request('gap_dig',target=[1,65,0],face='West')
    assert 'error' not in dug and dug['record']['removed'],dug
    known=sum(s is not None for s in dug['before']['inventory']['slots'])
    report['checks'].append(dict(name='hand_dig_received_selected_slot',known_slots=known,total_slots=len(dug['before']['inventory']['slots']),result=dug,native=command('execute if block 1 65 0 minecraft:air')))

    # Retire the dirt drop before the separate pose-control fixture. Item contact
    # prediction is outside this observation change and remains explicitly unsupported.
    command('kill @e[type=minecraft:item]')
    until(lambda: not observe()['items']['stacks'],10)

    # Actual client input -> server pose metadata -> independent eye-height calculation.
    command('fill -1 65 -1 3 67 1 minecraft:air')
    command('tp ClimbingProbe 0.5 65 0.5 0 0')
    request('prepare',position=[.5,65.,.5])
    if version=='1.21.11':
        # A vanilla server does not send an unchanged default scale. Establish
        # it through real changed attribute receipts instead of assuming one.
        command('attribute ClimbingProbe minecraft:scale base set 1.25')
        until(lambda: observe()['capture']['player']['attributes'].get('minecraft:scale',{}).get('value',{}).get('value')==1.25,5)
        command('attribute ClimbingProbe minecraft:scale base set 1')
        until(lambda: observe()['capture']['player']['attributes'].get('minecraft:scale',{}).get('value',{}).get('value')==1,5)
    start=request('mode_start',mode='survival')
    assert start['admitted'],start
    session=start['record']['session_id']
    for crouch,id,eye in [(True,5,1.27),(False,0,1.62)]:
        keys=dict(forward=0,strafe=0,jump=False,sneak=crouch,sprint=False,yaw=0.,pitch=0.)
        result=request('mode_keys',mode='survival',session_id=session,controls=keys)
        assert result['admitted'],result
        reading=until(lambda: (s:=observe()) and s['posture']['pose'] and s['posture']['pose']['value']==id and s,10)
        packet=receipt(reading['posture']['pose'],reading['capture']['player']['entity_id'])
        height=reading['posture']['eye_height']
        assert abs(height['height']['value']-eye)<1e-6,height
        assert height['height']['source']['kind']=='predicted',height
        if version=='1.21.11':
            assert height['scale']['source']['kind']=='received',height
        report['checks'].append(dict(name='received_crouching' if crouch else 'received_standing',observation=reading,packet=packet))
    stopped=request('mode_stop',mode='survival',session_id=session)
    assert stopped['admitted'],stopped

    # The server sends the shared swimming bit independently of the swimming pose.
    command('fill -2 65 -2 2 68 6 minecraft:water')
    command('tp ClimbingProbe 0.5 65 0.5 0 0')
    request('prepare',position=[.5,65.,.5])
    start=request('mode_start',mode='survival')
    assert start['admitted'],start
    session=start['record']['session_id']
    keys=dict(forward=1,strafe=0,jump=False,sneak=False,sprint=True,yaw=0.,pitch=0.)
    result=request('mode_keys',mode='survival',session_id=session,controls=keys)
    assert result['admitted'],result
    water=until(lambda: (s:=observe()) and s['posture']['swimming'] and s['posture']['swimming']['value'] and s,10)
    packet=receipt(water['posture']['swimming'],water['capture']['player']['entity_id'])
    assert water['predicted']['in_water']==dict(value=True,source=dict(kind='predicted')),water
    assert water['predicted']['swimming']['source']['kind']=='predicted',water
    report['checks'].append(dict(name='received_swimming_vs_predicted_water',observation=water,packet=packet,native=command('data get entity ClimbingProbe Pos')))
    assert request('mode_stop',mode='survival',session_id=session)['admitted']
    trace.expect_disconnect()
    request('disconnect')


class SparseInventoryProxy(PacketTraceProxy):
    """Withhold whole original inventory baselines; every delivered frame is unchanged.

    This is a separate receipt-loss trial, not the normal transparent live test.
    Original full frames stay recorded as explicitly undelivered evidence.
    """
    def __init__(self, *args):
        self.delivery=threading.local()
        self.withheld=[]
        super().__init__(*args)

    def before_forward(self, record):
        self.delivery.record=record

    def forward(self, source, destination, direction, state):
        proxy=self
        class Delivery:
            def sendall(self, data):
                record=proxy.delivery.record
                full=0x14 if proxy.version=='1.16.1' else 0x12
                legacy_cursor=(proxy.version=='1.16.1' and record['packet_id']==0x16 and record.get('body_hex','').startswith('ffffff'))
                if record['phase']=='play' and (record['packet_id']==full or legacy_cursor):
                    proxy.withheld.append(dict(record, delivered=False))
                    return
                destination.sendall(data)
            def shutdown(self, how):
                destination.shutdown(how)
        super().forward(source, Delivery() if direction=='clientbound' else destination, direction, state)


def sparse_check(version, command, request, trace, report):
    command('tp ClimbingProbe 0.5 65 0.5 0 0')
    request('prepare',position=[.5,65.,.5])
    region=dict(min=[1,65,0],max=[1,65,0])
    def observe():
        return request('observation_gaps',region=region)
    command('setblock 1 65 0 minecraft:dirt')
    until(lambda: observe()['capture']['world']['blocks'][0]['state']['name']=='minecraft:dirt',5)
    request('select_hotbar',slot=0)
    missing=request('gap_estimate',target=[1,65,0])
    assert missing.get('refusal')==dict(MissingSlot=dict(slot=36)),missing
    report['checks'].append(dict(name='typed_missing_selected_slot',result=missing))
    command('give ClimbingProbe minecraft:stick')
    until(lambda: (v:=observe()['capture']['player']['inventory']['slots'][36]) and v['value']['kind']=='item',5)
    command('clear ClimbingProbe')
    until(lambda: (v:=observe()['capture']['player']['inventory']['slots'][36]) and v['value']['kind']=='empty',5)
    dug=request('gap_dig',target=[1,65,0],face='West')
    assert 'error' not in dug and dug['record']['removed'],dug
    slots=dug['before']['inventory']['slots']
    assert slots[36]['value']['kind']=='empty' and sum(s is not None for s in slots)<46,slots
    assert slots[5] is None and dug['before']['inventory']['cursor'] is None,dug
    assert trace.withheld, 'the original inventory baseline was never withheld'
    report['checks'].append(dict(name='hand_dig_without_unrelated_slot_or_cursor_receipts',known_slots=sum(s is not None for s in slots),result=dug,native=command('execute if block 1 65 0 minecraft:air')))
    report['undelivered_inventory_baselines']=trace.withheld
    trace.expect_disconnect()
    request('disconnect')


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--accept-eula',action='store_true',required=True)
    parser.add_argument('--binary',type=Path,required=True)
    parser.add_argument('--jars',type=Path,required=True)
    parser.add_argument('--sparse-inventory',action='store_true',help='separate whole-baseline receipt-loss trial')
    parser.add_argument('--version',choices=['1.16.1','1.21.11'],action='append')
    parser.add_argument('--compiled-sdk-revision',help='require clean SDK sources identical to this build reference')
    args=parser.parse_args()
    if args.compiled_sdk_revision:
        paths=['src','examples','data','Cargo.toml','Cargo.lock']
        assert not subprocess.check_output(['git','diff',args.compiled_sdk_revision,'--',*paths],cwd=REPO)
        assert not subprocess.check_output(['git','ls-files','--others','--exclude-standard','--',*paths],cwd=REPO)
    binary=args.binary.resolve()
    for version in args.version or ['1.16.1','1.21.11']:
        def checked(*arguments):
            report=arguments[-1]
            report['sdk']=dict(compiled_sdk_revision=args.compiled_sdk_revision,revision=subprocess.check_output(['git','rev-parse','HEAD'],cwd=REPO,text=True).strip(),binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest())
            (sparse_check if args.sparse_inventory else check)(*arguments)
        run(version,binary,args.jars.resolve(),check=checked,trace_factory=SparseInventoryProxy if args.sparse_inventory else PacketTraceProxy)


if __name__=='__main__':
    main()
