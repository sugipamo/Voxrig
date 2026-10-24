#!/usr/bin/env python3
"""Group 3: finite natural walking and moving explicit placements on official servers.

Disposable loopback servers, SHA-1 verified JARs, unmodified-byte trace and RCON.
"""
import argparse
import hashlib
import json
import re
import struct
import subprocess
from pathlib import Path
from run_climbing_control import REPO, run
from run_common_native import until


def check(version, command, request, trace, report, scenario):
    report["scenario"]=scenario
    def prepare(pos=(.5,65.,.5)):
        command('tp ClimbingProbe %s %s %s 180 0'%tuple(pos))
        prepared=request('prepare',position=list(pos))
        assert 'error' not in prepared,prepared
        # Legacy deliberately parks its idle producer for 20 ticks after teleport.
        request('wait',ms=1400)
        return prepared
    def observe(*blocks):
        return request('observe',blocks=list(blocks))
    def native_position():
        native=command('data get entity ClimbingProbe Pos')
        return [float(s.strip().rstrip('df')) for s in re.search(r'\[([^]]+)\]',native)[1].split(',')]
    def known(pos,name):
        return until(lambda: (s:=next(b for b in observe(pos)['blocks'] if b['position']==pos)['state']) and s['value']['name']=='minecraft:'+name and s,5)
    def path(name,walking,expected_z,expected_y=65.,retreat=0):
        controls=[dict(yaw=0.,input=dict(forward=1,strafe=0,jump=False)) for _ in range(walking)]
        controls += [dict(yaw=0.,input=dict(forward=-1,strafe=0,jump=False)) for _ in range(retreat)]
        controls += [dict(yaw=0.,input=dict(forward=0,strafe=0,jump=False)) for _ in range(16)]
        boundary=trace.mark()
        started=request('path_start',controls=controls)
        assert 'error' not in started,(name,started)
        def done():
            record=request('path_record')
            assert 'error' not in record,record
            status=record['status']
            return record if status!='running' else None
        final=until(done,15)
        assert final['status']=='predicted',(name,final)
        if 'collides' in name:
            assert any(f['horizontal_collision'] for f in final['preview']['frames']),final
        pos=native_position()
        assert expected_z[0]<pos[2]<expected_z[1],(name,pos,final)
        assert abs(pos[1]-expected_y)<.02,(name,pos,final)
        predicted=final['preview']['frames'][-1]['position']
        assert max(abs(a-b) for a,b in zip(pos,predicted))<.02,(name,pos,predicted)
        report['checks'].append(dict(name=name,record=final,native_position=pos,frames=trace.since(boundary)))

    command('fill -6 63 -4 6 63 12 minecraft:stone')
    command('fill -6 64 -4 6 64 12 minecraft:sand')
    command('fill -6 65 -4 6 69 12 minecraft:air')
    for z,block in enumerate(['grass_block','dirt','grass_block','dirt','grass_block','sand','clay','gravel','oak_leaves','oak_log'],1):
        command('fill -1 64 %d 1 64 %d minecraft:%s'%(z,z,block))
    for z,block in [(1,'fern'),(2,'large_fern[half=lower]'),(3,'tall_grass[half=lower]'),(4,'poppy'),(5,'oak_sapling')]:
        command('setblock 0 65 %d minecraft:%s'%(z,block))
        if 'half=lower' in block:
            command('setblock 0 66 %d minecraft:%s'%(z,block.replace('lower','upper')))
    command('fill 2 65 1 2 67 7 minecraft:oak_log')
    command('fill -2 65 1 -2 67 7 minecraft:oak_leaves')
    # Each finite case gets its own connection: a server teleport deliberately
    # invalidates retained finite motion authority and must not reset that guard.
    if scenario=='natural':
        prepare()
        known([0,65,1],'fern')
        path('finite_path_through_fern_forest_sand_clay_gravel',44,(9.,11.))
    elif scenario=='log':
        command('fill -1 65 3 1 67 3 minecraft:oak_log')
        prepare()
        known([0,65,3],'oak_log')
        path('finite_path_collides_with_log_wall',25,(1.,2.6),retreat=6)
    elif scenario=='snow':
        command('fill -2 65 1 2 68 8 minecraft:air')
        command('fill -1 65 2 1 65 8 minecraft:snow[layers=4]')
        prepare()
        known([0,65,2],'snow')
        path('finite_path_steps_on_four_snow_layers',24,(4.,8.),65.375)

    if scenario=='bridge':
        assert version=='1.16.1'
        command('fill -3 65 -3 3 70 12 minecraft:air')
        # Remove the floor beyond the initial support: the bot supplies the bridge.
        command('fill -2 62 1 2 64 12 minecraft:air')
        command('setblock 0 64 0 minecraft:stone')
        command('replaceitem entity ClimbingProbe hotbar.0 minecraft:gold_block 16')
        prepare((.5,65.,.2))
        request('select',slot=0)
        until(lambda: isinstance((s:=observe()['player']['inventory'][36]),dict) and isinstance(s['value'],dict) and s['value'].get('Item',{}).get('name')=='minecraft:gold_block',5)
        # Looking north, clicking south: this intentionally fails the strict raycast API.
        strict=request('strict_place',pos=[0,64,0])
        assert 'error' in strict,strict
        report['checks'].append(dict(name='strict_cube_diagnostics_retained',result=strict))
        start=request('control_start')
        assert 'error' not in start,start
        controls=dict(forward=1,strafe=0,jump=False,sneak=True,sprint=False,yaw=0.,pitch=0.)
        moving=request('control_set',controls=controls)
        assert 'error' not in moving,moving
        before=native_position()
        bridge=[]
        for z in range(1,6):
            # Wait until within ordinary server reach while keeping the same input session.
            until(lambda: observe()['player']['position']['value'][2]>z-3.8,8)
            boundary=trace.mark()
            submitted=request('submit',kind='place',pos=[0,64,z-1],face='South',cursor=[.17,.6,1.],target=[0,64,z],expected=dict(name='minecraft:gold_block',properties={}))
            assert 'error' not in submitted,submitted
            def complete():
                value=request('status',id=submitted)
                assert 'error' not in value,value
                return value if value['status']!='pending' else None
            result=until(complete,12)
            assert result['status']=='accepted',(z,result)
            known([0,64,z],'gold_block')
            native=command('execute if block 0 64 %d minecraft:gold_block'%z)
            assert 'Test passed' in native,native
            frames=trace.since(boundary)
            uses=[p for p in frames if p['direction']=='serverbound' and p['phase']=='play' and p['packet_id']==0x2d]
            assert len(uses)==1,uses
            raw=bytes.fromhex(uses[0]['body_hex'])
            assert raw[0]==0 and raw[9]==3 and raw[10:22]==struct.pack('>fff',.17,.6,1.),raw.hex()
            assert any(p['direction']=='serverbound' and p['phase']=='play' and p['packet_id'] in (0x12,0x13) for p in frames),frames
            bridge.append(dict(target=[0,64,z],result=result,native=native,frames=frames))
            request('wait',ms=60) # allow action ownership cleanup before the next submit
        until(lambda: observe()['player']['position']['value'][2]>3.5,10)
        stopped=request('control_stop')
        assert 'error' not in stopped,stopped
        after=native_position()
        assert after[2]>before[2]+3 and abs(after[1]-65.)<.02,(before,after)
        assert stopped['session_id']==start['session_id'],(start,stopped)
        report['checks'].append(dict(name='five_block_bridge_during_one_continuous_sneaking_session',before=before,after=after,placements=bridge,start=start,stopped=stopped))
    trace.expect_disconnect()
    request('disconnect')


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--accept-eula',action='store_true',required=True)
    parser.add_argument('--binary',type=Path,required=True)
    parser.add_argument('--jars',type=Path,required=True)
    parser.add_argument('--version',choices=['1.16.1','1.21.11'],action='append')
    parser.add_argument('--compiled-sdk-revision',help='require SDK sources identical to the compiled reference')
    parser.add_argument('--scenario',choices=['natural','log','snow','bridge'],action='append')
    args=parser.parse_args()
    sdk_paths=['src','examples','data','Cargo.toml','Cargo.lock']
    if args.compiled_sdk_revision:
        assert not subprocess.check_output(['git','diff',args.compiled_sdk_revision,'--',*sdk_paths],cwd=REPO)
        assert not subprocess.check_output(['git','ls-files','--others','--exclude-standard','--',*sdk_paths],cwd=REPO)
    binary=args.binary.resolve()
    for version in args.version or ['1.16.1','1.21.11']:
        scenarios=args.scenario or (['natural','log','snow','bridge'] if version=='1.16.1' else ['natural','log','snow'])
        for scenario in scenarios:
            if version!='1.16.1' and scenario=='bridge':
                continue
            def checked(*arguments):
                arguments[-1]['sdk']=dict(compiled_sdk_revision=args.compiled_sdk_revision,revision=subprocess.check_output(['git','rev-parse','HEAD'],cwd=REPO,text=True).strip(),binary_sha256=hashlib.sha256(binary.read_bytes()).hexdigest())
                check(*arguments,scenario)
            run(version,binary,args.jars.resolve(),check=checked)


if __name__=='__main__':
    main()
