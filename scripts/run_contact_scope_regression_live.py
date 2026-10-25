#!/usr/bin/env python3
"""Contact-scope regression on unmodified official loopback servers (native binary)."""
import argparse
import hashlib
import json
import re
from pathlib import Path
from run_climbing_control import REPO, run
from run_common_native import until


def check(version, command, request, trace, report):
    modern = version == '1.21.11'
    report['sdk'] = dict(binary_sha256=hashlib.sha256(BINARY.read_bytes()).hexdigest(),
        source_sha256={str(p.relative_to(REPO)): hashlib.sha256(p.read_bytes()).hexdigest()
                      for p in sorted([*(REPO/'src').rglob('*.rs'), REPO/'examples/climbing_control_probe.rs', Path(__file__).resolve()])})
    command('difficulty normal')
    command('time set midnight')
    command('gamerule '+('minecraft:mob_drops' if modern else 'doMobLoot')+' false')
    command('fill -8 64 -8 24 64 64 minecraft:stone')
    command('forceload add 32 0 64 16')
    command('fill 32 64 0 64 64 16 minecraft:stone')
    command('tp ClimbingProbe .5 65 .5 0 0')
    request('prepare', position=[.5, 65., .5])
    def names():
        return [e['motion']['entity']['type_name'] for e in request('entities')['entities']]
    for name,x in [('fox',40),('skeleton',48),('chest_minecart',56)]:
        command(f'summon minecraft:{name} {x} 65 .5 {{Tags:["RegressionFar"],NoAI:1b,NoGravity:1b,Invulnerable:1b}}')
    item = '{id:"minecraft:stone",Count:1b}' if not modern else '{id:"minecraft:stone",count:1}'
    for z in [0,2,4,6]:
        command(f'summon minecraft:item .5 65 {z}.5 {{Tags:["RegressionItem"],NoGravity:1b,PickupDelay:32767s,Item:{item}}}')
    until(lambda: all('minecraft:'+n in names() for n in ['fox','skeleton','chest_minecart','item']), 15)
    before = command('data get entity ClimbingProbe Pos')
    start = request('mode_start', mode='survival')
    assert start['admitted'], start
    owner = start['record']['session_id']
    keys=dict(forward=1,strafe=0,jump=False,sneak=False,sprint=False,yaw=0.,pitch=0.)
    assert request('mode_keys',mode='survival',session_id=owner,controls=keys)['admitted']
    moving=request('ticks',count=12)
    assert moving['status']['status']=='running' and moving['session_id']==owner,moving
    after=command('data get entity ClimbingProbe Pos')
    pos=lambda raw: [float(v.strip().rstrip('df')) for v in re.search(r'\[([^]]+)\]',raw)[1].split(',')]
    assert pos(after)[2]>pos(before)[2]+.5,(before,after)
    report['checks'].append(dict(name='far_fox_skeleton_cart_and_adjacent_drops',start=start,record=moving,native_before=before,native_after=after))
    # Same owner, repeated nearby spawn/despawn with real constructor/metadata/removal packets.
    neutral=dict(keys,forward=0)
    assert request('mode_keys',mode='survival',session_id=owner,controls=neutral)['admitted']
    records=[]
    for i in range(8):
        command('summon minecraft:cow 6.5 65 5.5 {Tags:["RegressionChurn"],NoAI:1b,NoGravity:1b,Invulnerable:1b}')
        until(lambda: 'minecraft:cow' in names(),10)
        record=request('ticks',count=5)
        assert record['status']['status']=='running' and record['session_id']==owner,record
        records.append(record)
        command('kill @e[tag=RegressionChurn]')
        until(lambda: 'minecraft:cow' not in names(),10)
        record=request('ticks',count=3)
        assert record['status']['status']=='running' and record['session_id']==owner,record
    report['checks'].append(dict(name='eight_near_spawn_despawn_cycles_same_owner',records=records))
    assert request('mode_stop',mode='survival',session_id=owner)['admitted']
    command('kill @e[tag=RegressionFar]')
    command('kill @e[tag=RegressionItem]')
    until(lambda: not names(),10)
    # Nonzero held input: server respawn paths differ with keepInventory. We
    # actually reassert the input; no cached selection is promoted to Received.
    command('gamerule '+('minecraft:keep_inventory' if modern else 'keepInventory')+' true')
    command('spawnpoint ClimbingProbe 0 65 0')
    request('select_hotbar',slot=5)
    boundary=trace.mark()
    command('kill ClimbingProbe')
    until(lambda: (p:=request('player'))['health'] and p['health']['value']['health']<=0,10)
    old=request('player')['session']['world_generation']
    respawn=request('respawn')
    until(lambda: (p:=request('player'))['session']['world_generation']>old and p['health'] and p['health']['value']['health']>0,15)
    selection=until(lambda: (p:=request('player'))['selected_hotbar'] and p['selected_hotbar']['value']==5 and p,10)
    assert selection['selected_hotbar']['source']['kind']=='submitted',selection
    # Fresh actual correction and loaded floor after respawn, then a real dig.
    command('tp ClimbingProbe .5 65 .5 0 0')
    request('prepare',position=[.5,65.,.5])
    if modern:
        command('attribute ClimbingProbe minecraft:scale base set 1.25')
        until(lambda: request('player')['attributes'].get('minecraft:scale',{}).get('value',{}).get('value')==1.25,10)
        command('attribute ClimbingProbe minecraft:scale base set 1')
        until(lambda: request('player')['attributes'].get('minecraft:scale',{}).get('value',{}).get('value')==1,10)
    command('give ClimbingProbe minecraft:stick')
    command('clear ClimbingProbe')
    request('wait',ms=200)
    settle=request('mode_start',mode='survival')
    assert settle['admitted'],settle
    sid=settle['record']['session_id']
    request('mode_keys',mode='survival',session_id=sid,controls=neutral)
    settled=request('ticks',count=20)
    assert settled['status']['status']=='running',settled
    assert request('mode_stop',mode='survival',session_id=sid)['admitted']
    if modern:
        # Standing actions require a received zero-velocity origin, separately
        # from the continuous controller's submitted pose. Keep that gate strict.
        old_pose=request('player')['received_pose']['receive_sequence']
        command('tp ClimbingProbe .5 65 .5 0 0')
        until(lambda: request('player')['received_pose']['receive_sequence']>old_pose,10)
        request('prepare',position=[.5,65.,.5])
    command('setblock 1 65 0 minecraft:dirt')
    request('wait',ms=150)
    dig=request('gap_dig',target=[1,65,0],face='West')
    assert 'error' not in dig and dig['record']['removed'],dig
    frames=trace.since(boundary)
    held=0x24 if not modern else 0x34
    spawn_id=0x3a if not modern else 0x50
    spawn_frame=next(f for f in frames if f['direction']=='clientbound'
                     and f['phase']=='play' and f['packet_id']==spawn_id)
    assert any(f['direction']=='serverbound' and f['phase']=='play'
               and f['ordinal']>spawn_frame['ordinal']
               and f['packet_id']==held and f['body_hex']=='0005' for f in frames), frames
    report['checks'].append(dict(name='respawn_nonzero_selection_then_dig_without_manual_reselection',respawn=respawn,
        selection=selection['selected_hotbar'],dig=dig,native=command('execute if block 1 65 0 minecraft:air'),frames=frames))
    trace.expect_disconnect()
    request('disconnect')


if __name__ == '__main__':
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--accept-eula',action='store_true',required=True)
    p.add_argument('--binary',type=Path,required=True)
    p.add_argument('--jars',type=Path,required=True)
    p.add_argument('--version',choices=['1.16.1','1.21.11'],required=True)
    args=p.parse_args()
    BINARY=args.binary.resolve()
    run(args.version,BINARY,args.jars.resolve(),check=check,
        server_properties={'view-distance':10,'simulation-distance':10})
