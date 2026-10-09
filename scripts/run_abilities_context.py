#!/usr/bin/env python3
"""Common ability/difficulty receipts from unchanged official servers."""
import argparse
import functools
import hashlib
from pathlib import Path
import re
import struct
import subprocess
from run_climbing_control import REPO,run
from run_common_native import until


def verify(version,context,trace):
    peers=[p for p in trace.since(0) if p['direction']=='clientbound' and p['phase'] in ('configuration','play')]
    resets={0x25,0x3a} if version=='1.16.1' else {0x30,0x50}
    world_boundary=max((i for i,p in enumerate(peers,1) if i<=context['receive_sequence'] and p['phase']=='play' and p['packet_id'] in resets),default=0)
    originals=[]
    for name in ('abilities','difficulty'):
        receipt=context[name]
        expected={'abilities':0x31 if version=='1.16.1' else 0x3e,'difficulty':0x0d if version=='1.16.1' else 0x0a}[name]
        candidates=[i for i,p in enumerate(peers,1) if world_boundary<i<=context['receive_sequence'] and p['phase']=='play' and p['packet_id']==expected]
        if receipt is None:
            if candidates:raise RuntimeError('applied original context packet became missing')
            continue
        source=receipt['source']
        if not candidates or source['sequence']!=candidates[-1]:
            raise RuntimeError('context is not the latest applied original same-world receipt')
        if source['kind']!='received' or not 0<source['sequence']<=context['receive_sequence']:
            raise RuntimeError('invalid original context source')
        packet=peers[source['sequence']-1]
        if packet['phase']!='play' or packet['packet_id']!=expected:
            raise RuntimeError('context source points to another original packet')
        raw=bytes.fromhex(packet['body_hex']);actual=receipt['value']
        if name=='abilities':
            decoded=struct.unpack('>Bff',raw)
            if raw!=struct.pack('>Bff',actual['flags'],actual['flying_speed'],actual['walking_speed']):
                raise RuntimeError('abilities differ from original native fields')
        elif len(raw)!=2 or raw[0]!=actual['id'] or bool(raw[1])!=actual['locked']:
            raise RuntimeError('difficulty differs from original native fields')
        originals.append(dict(field=name,receipt=receipt,original=packet))
    return originals


def check(version,command,request,trace,report,*,sdk):
    report['sdk']=sdk
    def capture(name):
        context=request('player_context')
        report['checks'].append(dict(name=name,context=context,originals=verify(version,context,trace)))
        return context
    def native_abilities():
        text=command('data get entity ClimbingProbe abilities')
        result={}
        for key in ('invulnerable','flying','mayfly','instabuild','flySpeed','walkSpeed'):
            match=re.search(r'\b'+key+r': ([+-]?[0-9.]+)[bf]?',text)
            if match is None:raise RuntimeError('native ability field missing: '+key)
            result[key]=float(match[1])
        return dict(flags=sum(int(result[k])*(1<<i) for i,k in enumerate(('invulnerable','flying','mayfly','instabuild'))),
                    flying_speed=result['flySpeed'],walking_speed=result['walkSpeed'])
    def matches(value,native):
        return value['flags']==native['flags'] and all(struct.pack('>f',value[k])==struct.pack('>f',native[k]) for k in ('flying_speed','walking_speed'))
    until(lambda: (c if (c:=request('player_context'))['abilities'] is not None and c['difficulty'] is not None else None),10)
    initial=capture('initial_received_abilities_and_difficulty')
    if initial['abilities'] is None or initial['abilities']['value']['flags']!=0 or initial['difficulty']['value']['locked']:
        raise RuntimeError('survival known-zero abilities were not retained')
    for mode in ('creative','spectator','survival'):
        before=request('player_context');command('gamemode '+mode+' ClimbingProbe');native=native_abilities()
        def arrived():
            context=request('player_context');receipt=context['abilities']
            return context if receipt is not None and receipt['source']['sequence']>before['receive_sequence'] and matches(receipt['value'],native) else None
        until(arrived,5);capture('received_'+mode+'_abilities');report['checks'][-1]['native']=native
    for name,identifier in (('hard',3),('peaceful',0),('easy',1),('normal',2)):
        before=request('player_context');result=command('difficulty '+name)
        def arrived():
            context=request('player_context');receipt=context['difficulty']
            return context if receipt is not None and receipt['source']['sequence']>before['receive_sequence'] and receipt['value']['id']==identifier else None
        until(arrived,5);capture('received_'+name+'_difficulty');report['checks'][-1]['native_command']=result
    before=request('player_context');command('experience add ClimbingProbe 1 points')
    until(lambda:(c if (c:=request('player_context'))['receive_sequence']>before['receive_sequence'] else None),5)
    unchanged=capture('unrelated_receipt_keeps_ability_and_difficulty_sources')
    if any(unchanged[k]!=before[k] for k in ('abilities','difficulty')):raise RuntimeError('unrelated packet refreshed unchanged context fields')
    old_generation=unchanged['session']['world_generation']
    boundary=trace.mark();command('execute in minecraft:the_nether run tp ClimbingProbe 0.5 80 0.5 0 0')
    until(lambda:(c if (c:=request('player_context'))['session']['world_generation']!=old_generation else None),10)
    request('wait',ms=150);current=capture('new_world_actual_receipts_or_absence')
    peers=[p for p in trace.since(0) if p['direction']=='clientbound' and p['phase'] in ('configuration','play')]
    respawns=[p for p in trace.since(boundary) if p['direction']=='clientbound' and p['phase']=='play' and p['packet_id']==(0x3a if version=='1.16.1' else 0x50)]
    if len(respawns)!=1:raise RuntimeError('native world replacement boundary missing or ambiguous')
    report['checks'][-1]['original_world_boundary']=respawns[0]
    for name in ('abilities','difficulty'):
        receipt=current[name]
        if receipt is not None and peers[receipt['source']['sequence']-1]['ordinal']<=respawns[0]['ordinal']:
            raise RuntimeError('old world receipt survived world replacement')
    command('gamemode creative ClimbingProbe');native=native_abilities()
    until(lambda:(c if (c:=request('player_context'))['abilities'] is not None and matches(c['abilities']['value'],native) else None),5)
    current=capture('new_world_fresh_abilities')
    trace.expect_disconnect();closed=request('context_disconnect')
    if closed['session']!=current['session'] or closed['receive_sequence']<current['receive_sequence']:
        raise RuntimeError('closed context lost its original world/capture boundary')
    # Original notifications can arrive between the preceding capture and close.
    # verify() requires the latest actually applied same-world packet, including
    # duplicate values; a fresh source is not mistaken for lost retained data.
    report['checks'].append(dict(name='received_context_readable_after_close',context=closed,originals=verify(version,closed,trace)))


def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--accept-eula',action='store_true',required=True)
    p.add_argument('--version',choices=('1.16.1','1.21.11'),action='append')
    p.add_argument('--binary',type=Path,required=True)
    p.add_argument('--jars',type=Path,default=REPO/'.local/climbing/downloads')
    p.add_argument('--compiled-sdk-revision',required=True)
    args=p.parse_args();revision=subprocess.check_output(['git','rev-parse','HEAD'],cwd=REPO,text=True).strip()
    compiled=subprocess.check_output(['git','rev-parse',args.compiled_sdk_revision],cwd=REPO,text=True).strip()
    if subprocess.check_output(['git','diff',compiled,'HEAD','--','src','examples','Cargo.toml','Cargo.lock'],cwd=REPO):
        raise RuntimeError('declared compiled SDK differs from current runtime source')
    sdk=dict(source_revision=revision,binary_build_revision=compiled,
             source_diff_sha256=hashlib.sha256(subprocess.check_output(['git','diff','HEAD'],cwd=REPO)).hexdigest(),
             binary_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest(),
             artifacts={f:hashlib.sha256((REPO/f).read_bytes()).hexdigest() for f in ('Cargo.lock','scripts/run_abilities_context.py','scripts/run_climbing_control.py','scripts/run_common_native.py','examples/climbing_control_probe.rs')})
    for version in args.version or ('1.16.1','1.21.11'):
        run(version,args.binary.resolve(),args.jars.resolve(),check=functools.partial(check,sdk=sdk))

if __name__=='__main__':main()
