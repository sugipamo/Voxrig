#!/usr/bin/env python3
"""All-type contact exclusion on unmodified official loopback servers (native SDK)."""
import argparse
import hashlib
import re
from pathlib import Path
from run_climbing_control import REPO, run
from run_common_native import until


def check(version, command, request, trace, report):
    modern=version=='1.21.11'
    report['sdk']=dict(binary_sha256=hashlib.sha256(BINARY.read_bytes()).hexdigest(),
        source_sha256={str(p.relative_to(REPO)):hashlib.sha256(p.read_bytes()).hexdigest()
                      for p in sorted([*(REPO/'src').rglob('*.rs'),REPO/'Cargo.toml',REPO/'Cargo.lock',
                                       REPO/'examples/climbing_control_probe.rs',Path(__file__).resolve()])})
    command('difficulty normal')
    command('time set midnight')
    command('gamerule '+('minecraft:mob_drops' if modern else 'doMobLoot')+' false')
    command('forceload add 40 -8 58 14')
    command('fill -8 64 -12 40 64 24 minecraft:stone')
    command('fill 40 64 -8 58 64 14 minecraft:stone')
    boat='oak_boat' if modern else 'boat'
    varied=['spider','cave_spider','slime','magma_cube','bee','iron_golem','phantom','bat','squid',
            boat,'armor_stand','cat','zombie_villager','witch','pillager','dolphin','turtle','llama',
            'panda','polar_bear','chest_minecart','furnace_minecart','tnt_minecart']
    if modern:
        varied+=['glow_squid','goat','axolotl','frog','camel','armadillo']
    def entities():return request('entities')['entities']
    def names():return [e['motion']['entity']['type_name'] for e in entities()]
    def position(raw):return [float(v.strip().rstrip('df')) for v in re.search(r'\[([^]]+)\]',raw)[1].split(',')]
    keys=dict(forward=1,strafe=0,jump=False,sneak=False,sprint=False,yaw=0.,pitch=0.)
    for phase,x in [('far',40.),('nearby_non_touching',18.)]:
        command('tp ClimbingProbe .5 65 .5 0 0')
        request('prepare',position=[.5,65.,.5])
        # No falling blocks are used in either walking set. Every candidate is
        # an actual official entity; NoAI/NoGravity isolate the received model lane.
        for i,name in enumerate(varied):
            z=-8.+(i%6)*4.
            y=66. if name in ['phantom','bat','bee','squid','glow_squid','dolphin'] else 65.
            reply=command(f'summon minecraft:{name} {x+(i//6)*4.} {y} {z} {{Tags:["ScopeAll"],NoAI:1b,NoGravity:1b,Invulnerable:1b,PersistenceRequired:1b,Size:0}}')
            assert 'Summoned' in reply,(name,reply)
        report.setdefault('native_origins',[]).append({n:command(f'data get entity @e[tag=ScopeAll,type=minecraft:{n},limit=1] Pos') for n in ['witch','goat'] if n in varied})
        until(lambda: all('minecraft:'+n in names() for n in varied),30)
        request('wait',ms=250)
        relationships=command('execute if entity @e[tag=ScopeAll,nbt={Passengers:[{}]}]')
        assert 'Test passed' not in relationships,relationships
        before=command('data get entity ClimbingProbe Pos');mark=trace.mark()
        report.setdefault('origins',[]).append(dict(phase=phase,player=request('player'),entities=entities()))
        started=request('mode_start',mode='survival');assert started['admitted'],started
        owner=started['record']['session_id']
        assert request('mode_keys',mode='survival',session_id=owner,controls=keys)['admitted']
        moving=request('ticks',count=12)
        assert moving['status']['status']=='running' and moving['session_id']==owner,moving
        after=command('data get entity ClimbingProbe Pos')
        assert position(after)[2]>position(before)[2]+.5,(before,after)
        assert request('mode_keys',mode='survival',session_id=owner,controls=dict(keys,forward=0))['admitted']
        settled=request('ticks',count=20);assert settled['status']['status']=='running',settled
        assert request('mode_stop',mode='survival',session_id=owner)['admitted']
        report['checks'].append(dict(name=phase+'_varied_official_registry_entities',summoned=varied,
            received=entities(),passenger_check=relationships,start=started,record=moving,native_before=before,native_after=after,frames=trace.since(mark)))
        command('tp @e[tag=ScopeAll] 10000 65 10000')
        until(lambda:not names(),20)
        command('kill @e[tag=ScopeAll]')
    # Touching an unsupported contact must remain typed and stop before a new
    # controlled movement owner/tick is admitted, even after successful walking.
    command('tp ClimbingProbe .5 65 .5 0 0')
    request('prepare',position=[.5,65.,.5])
    command('summon minecraft:spider .9 65 .5 {Tags:["ScopeTouch"],NoAI:1b,NoGravity:1b,Invulnerable:1b}')
    until(lambda:'minecraft:spider' in names(),15)
    request('wait',ms=100);mark=trace.mark()
    failed=request('mode_start',mode='survival')
    assert not failed['admitted'],failed
    refusal=failed['movement_refusal']['Scoped']
    assert refusal['refusal']['Unsupported']['context']['entity_type']=='minecraft:spider',refusal
    assert refusal['geometry']['entity_position'] is not None,refusal
    report['checks'].append(dict(name='touching_spider_still_typed_refusal',refusal=failed,frames=trace.since(mark)))
    command('tp @e[tag=ScopeTouch] 10000 65 10000')
    until(lambda:not names(),15)
    command('tp ClimbingProbe .5 65 .5 0 0');request('prepare',position=[.5,65.,.5])
    restarted=request('mode_start',mode='survival');assert restarted['admitted'],restarted
    request('mode_stop',mode='survival',session_id=restarted['record']['session_id'])
    report['checks'].append(dict(name='same_connection_recovers_after_touching_cause_leaves',start=restarted))
    trace.expect_disconnect();request('disconnect')


if __name__=='__main__':
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--accept-eula',action='store_true',required=True)
    p.add_argument('--binary',type=Path,required=True)
    p.add_argument('--jars',type=Path,required=True)
    p.add_argument('--version',choices=['1.16.1','1.21.11'],required=True)
    args=p.parse_args();BINARY=args.binary.resolve()
    run(args.version,BINARY,args.jars.resolve(),check=check,server_properties={'view-distance':10,'simulation-distance':10})
