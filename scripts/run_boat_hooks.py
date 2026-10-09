#!/usr/bin/env python3
"""Common nonliving boat terrain callbacks against unchanged official servers."""
import argparse
import functools
import hashlib
import json
from pathlib import Path
import re
import struct
import subprocess

from run_boat_bubbles import velocity_packet
from run_climbing_control import REPO, run
from run_common_native import until


def check(version, command, request, trace, report, *, sdk):
    report['sdk']=sdk
    command('gamerule '+('randomTickSpeed' if version=='1.16.1' else 'minecraft:random_tick_speed')+' 0')
    boat_type='minecraft:boat' if version=='1.16.1' else 'minecraft:oak_boat'
    selector='@e[tag=TerrainMount,limit=1]'
    move_id,paddle_id=(0x16,0x17) if version=='1.16.1' else (0x21,0x22)
    velocity_id=0x46 if version=='1.16.1' else 0x63
    def native(field='Pos'):
        value=command('data get entity '+selector+' '+field)
        match=re.search(r'\[([^]]+)\]',value)
        if match is None: raise RuntimeError('original native vector missing: '+value)
        return [float(v.strip().rstrip('df')) for v in match[1].split(',')]
    for kind in ('slime','bed','cobweb','honey'):
        command('kill @e[tag=TerrainMount]')
        command('fill -8 63 -8 8 82 8 minecraft:air')
        command('fill -8 63 -8 8 64 8 minecraft:stone')
        command('tp ClimbingProbe 0.5 65 -0.5 0 0')
        request('prepare',position=[0.5,65.0,-0.5])
        height=65.25
        if kind=='bed':
            command('fill -8 64 -8 8 64 8 minecraft:air')
            command('give ClimbingProbe minecraft:red_bed 1')
            until(lambda: (s if (s:=request('player')['inventory']['slots'][36]) is not None
                          and s['value']['kind']=='item' else None),10)
            request('use_on_block',support=[0,63,1],face='Up',cursor=[0.5,1.0,0.5])
            until(lambda: 'Test passed' in command('execute if block 0 64 1 minecraft:red_bed[part=foot,facing=south]'),10)
            if 'Test passed' not in command('execute if block 0 64 2 minecraft:red_bed[part=head,facing=south]'):
                raise RuntimeError('official bed item did not place the complete pair')
        elif kind=='slime':
            command('fill -8 64 -8 8 64 8 minecraft:slime_block')
        elif kind=='honey':
            command('fill -8 64 -8 8 64 8 minecraft:honey_block')
        command('summon '+boat_type+' 0.5 '+str(height)+' 1.5 {Tags:["TerrainMount"],Invulnerable:1b}')
        mounted=request('mount',type=boat_type)
        report['checks'].append(dict(name=kind+'_mount',received=mounted,native=command('data get entity ClimbingProbe RootVehicle.Attach')))
        if kind=='cobweb':
            command('fill -3 65 -3 3 66 6 minecraft:cobweb')
        request('wait',ms=150)
        boundary=trace.mark()
        before=native()
        inputs=[dict(forward=1,strafe=0,jump=False)]*35+[dict(forward=0,strafe=0,jump=False)]*5
        if kind=='honey':
            outcome=request('drive_cancelled_waiter',inputs=inputs)
            record=outcome['record']
            if not outcome.get('cancelled_waiter') or outcome['pending']['stage']!='running':
                raise RuntimeError('cancelled hook waiter lost its original running owner')
        else:
            record=request('drive',inputs=inputs)
            outcome=dict(record=record)
        if record['stage']!='submitted' or record['dispatched_ticks']!=len(inputs):
            raise RuntimeError('audited hook finite run did not complete: '+str(record.get('requires_inspection')))
        frames=record['boat_motion']['frames']
        def received_writes():
            packets=trace.since(boundary)
            moves=[p for p in packets if p['direction']=='serverbound' and p['phase']=='play' and p['packet_id']==move_id]
            paddles=[p for p in packets if p['direction']=='serverbound' and p['phase']=='play' and p['packet_id']==paddle_id]
            return (moves,paddles) if len(moves)>=len(frames) and len(paddles)>=len(frames) else None
        moves,paddles=until(received_writes)
        if len(moves)!=len(frames) or len(paddles)!=len(frames):
            raise RuntimeError('terrain boat dispatch history/wire counts differ')
        for frame,move,paddle in zip(frames,moves,paddles):
            original=struct.pack('>dddff',*frame['position'],*frame['rotation'])
            if version!='1.16.1':original+=bytes([frame['on_ground']])
            if move['body_hex']!=original.hex() or paddle['body_hex']!=bytes(frame['paddles']).hex():
                raise RuntimeError('original boat write differs from retained terrain model')
        report['checks'].append(dict(name=kind+'_finite_dispatch',outcome=outcome,native_before=before,writes=moves+paddles))
        if kind in ('slime','bed') and not any(f['on_ground'] and f['velocity'][1]>0 for f in frames):
            raise RuntimeError('native nonliving landing bounce was not exercised')
        if kind=='cobweb' and not any(f['stuck']==[0.25,0.05000000074505806,0.25] for f in frames):
            raise RuntimeError('native deferred cobweb multiplier was not exercised')
        peers=[p for p in trace.since(0) if p['direction']=='clientbound' and p['phase'] in ('configuration','play')]
        sources=[]
        for update in record['boat_motion']['velocity_updates']:
            source=update['receipt']['source']; original=peers[source['sequence']-1]
            allowed=(velocity_id,) if version=='1.16.1' else (velocity_id,0x23)
            if source['kind']!='received' or original['phase']!='play' or original['packet_id'] not in allowed:
                raise RuntimeError('terrain velocity source points to another original packet')
            target,velocity=velocity_packet(version,original)
            if target!=record['id']['mount']['vehicle_native_id'] or velocity!=update['receipt']['value']:
                raise RuntimeError('terrain velocity receipt differs from original native fields')
            sources.append(dict(update=update,original=original))
        report['checks'].append(dict(name=kind+'_velocity_sources',verified=sources))
        position=until(lambda: (p if max(abs(a-b) for a,b in zip((p:=native()),frames[-1]['position']))<0.4 else None),5)
        report['checks'].append(dict(name=kind+'_native_endpoint',position=position,predicted=frames[-1]['position'],motion=native('Motion')))
        dismount=request('dismount')
        if dismount['stage']!='completed':raise RuntimeError('post-hook dismount did not complete')
        report['checks'].append(dict(name=kind+'_received_dismount',record=dismount))
    trace.expect_disconnect()
    request('disconnect')


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--accept-eula',action='store_true',required=True)
    parser.add_argument('--version',choices=('1.16.1','1.21.11'),action='append')
    parser.add_argument('--binary',type=Path,required=True)
    parser.add_argument('--jars',type=Path,default=REPO/'.local/climbing/downloads')
    parser.add_argument('--compiled-sdk-revision',required=True)
    args=parser.parse_args()
    revision=subprocess.check_output(['git','rev-parse','HEAD'],cwd=REPO,text=True).strip()
    compiled=subprocess.check_output(['git','rev-parse',args.compiled_sdk_revision],cwd=REPO,text=True).strip()
    if subprocess.check_output(['git','diff',compiled,'HEAD','--','src','examples','Cargo.toml','Cargo.lock'],cwd=REPO):
        raise RuntimeError('declared compiled SDK differs from current runtime source')
    sdk=dict(source_revision=revision,binary_build_revision=compiled,
             binary_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest(),
             source_diff_sha256=hashlib.sha256(subprocess.check_output(['git','diff','HEAD'],cwd=REPO)).hexdigest(),
             artifacts={p:hashlib.sha256((REPO/p).read_bytes()).hexdigest() for p in (
                 'Cargo.lock','scripts/run_boat_hooks.py','scripts/run_boat_bubbles.py','scripts/run_climbing_control.py',
                 'scripts/run_common_native.py','examples/climbing_control_probe.rs','scripts/movement_oracle/run.py',
                 'scripts/movement_oracle/boat_hook_scenarios.json','scripts/movement_oracle/java_1_16_1/MovementOracle.java',
                 'scripts/movement_oracle/java_1_21_11/MovementOracle.java','data/client_api/boat_hooks_oracle.json.gz')})
    for version in args.version or ('1.16.1','1.21.11'):
        run(version,args.binary.resolve(),args.jars.resolve(),check=functools.partial(check,sdk=sdk))


if __name__=='__main__':main()
