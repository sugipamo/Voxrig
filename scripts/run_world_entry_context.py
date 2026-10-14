#!/usr/bin/env python3
"""World entry, cooldown identity and context events on unchanged official servers."""
import argparse
import functools
import hashlib
from pathlib import Path
import re
import struct
import subprocess
from run_climbing_control import REPO, run
from run_common_native import NativeSocialReader, until
from run_player_context import unpack_position


def entry(version, packet):
    r = NativeSocialReader(packet, version)
    old = version == '1.16.1'
    login = packet['packet_id'] == (0x25 if old else 0x30)
    conditions = None
    if login:
        r.take(4)
        if old:
            mode, previous = r.take(2)
        else:
            hardcore = r.boolean()
        for _ in range(r.integer()): r.string()
        if old:
            tag = r.take(1)[0]; r.take(int.from_bytes(r.take(2), 'big')); r.nbt(tag)
        else:
            maximum, view, simulation = r.integer(), r.integer(), r.integer()
            reduced, respawn, limited = r.boolean(), r.boolean(), r.boolean()
    if old:
        dimension_name, world = r.string(), r.string()
        seed = struct.unpack('>q', r.take(8))[0]
        if login:
            maximum = r.take(1)[0]; view = r.integer()
            reduced, respawn = r.boolean(), r.boolean()
        else:
            mode, previous = r.take(2)
        debug, flat = r.boolean(), r.boolean()
        death = portal = sea = dimension_id = None
    else:
        dimension_id, world = r.integer(), r.string()
        seed = struct.unpack('>q', r.take(8))[0]
        mode, previous = r.take(2)
        debug, flat = r.boolean(), r.boolean()
        death = dict(dimension=r.string(), position=unpack_position(r.take(8))) if r.boolean() else None
        portal, sea = r.integer(), r.integer()
        dimension_name = None
    keep = r.take(1)[0] if not login else None
    if login:
        secure = r.boolean() if not old else None
        conditions = dict(max_players=maximum, reduced_debug_info=reduced, enable_respawn_screen=respawn,
                          hardcore=None if old else hardcore, limited_crafting=None if old else limited,
                          enforces_secure_chat=secure)
    if r.cursor != len(r.raw): raise RuntimeError('unconsumed native world-entry fields')
    return dict(world_name=world, dimension_type_name=dimension_name, dimension_type_id=dimension_id,
                game_mode=mode, previous_game_mode=previous if previous < 128 else previous-256,
                hashed_seed=seed, debug=debug, flat=flat, last_death=death, portal_cooldown=portal,
                sea_level=sea, respawn_keep_data=keep), conditions


def verify(version, context, trace):
    peers = [p for p in trace.since(0) if p['direction']=='clientbound' and p['phase'] in ('configuration','play')]
    receipt = context['world_entry']
    if receipt is None: raise RuntimeError('complete original world entry missing')
    sequence = receipt['source']['sequence']; packet = peers[sequence-1]
    expected = (0x25,0x3a) if version=='1.16.1' else (0x30,0x50)
    if packet['phase']!='play' or packet['packet_id'] not in expected: raise RuntimeError('world entry source mismatch')
    value, conditions = entry(version,packet)
    if value != receipt['value']: raise RuntimeError('world-entry receipt differs from native packet')
    actual = context['login_conditions']
    if (actual['value'] if actual else None) != conditions: raise RuntimeError('LOGIN-only conditions differ or survive respawn')
    if actual and actual['source'] != receipt['source']: raise RuntimeError('LOGIN conditions source mismatch')
    latest = {}
    cooldown_id = 0x17 if version=='1.16.1' else 0x16
    for i,p in enumerate(peers,1):
        if sequence < i <= context['receive_sequence'] and p['phase']=='play' and p['packet_id']==cooldown_id:
            r=NativeSocialReader(p,version); key=r.integer() if version=='1.16.1' else r.string();ticks=r.integer()
            if r.cursor!=len(r.raw):raise RuntimeError('native cooldown has trailing data')
            latest[key]=(ticks,i)
    actual_cooldowns={c['key']['value']:(c['ticks']['value'],c['ticks']['source']['sequence']) for c in context['item_cooldowns']}
    if actual_cooldowns != latest:raise RuntimeError('cooldown identity/ticks/source or world ownership differ')
    return dict(original_entry=packet, cooldown_count=len(latest))


def check(version, command, request, trace, report, *, sdk):
    report['sdk']=sdk
    cursor=0
    def capture(name):
        nonlocal cursor
        context=request('player_context'); original=verify(version,context,trace)
        events=request('events',cursor=cursor);cursor=events['cursor']
        report['checks'].append(dict(name=name,context=context,original=original,events=events))
        return context,events
    def notified(context,events,receipt):
        source=receipt['source']['sequence']
        if not any(e['kind']=='ContextChanged' and e['receive_sequence']==source for e in events['events']):
            raise RuntimeError('applied context receipt lacks its original-source notification')
    first,events=capture('original_login_conditions_and_previous_mode')
    notified(first,events,first['world_entry'])
    if first['world_entry']['value']['previous_game_mode'] != -1:raise RuntimeError('first login unknown previous mode changed')
    command('tp ClimbingProbe 0.5 65 0.5 0 -80')
    request('prepare',position=[0.5,65.,0.5])
    command('give ClimbingProbe minecraft:ender_pearl 4')
    until(lambda: any(s['value']['kind']=='item' and s['value']['item']['name']=='minecraft:ender_pearl' for s in request('player')['inventory']['slots'] if s),5)
    before=request('player_context')['receive_sequence']; request('use_item')
    def received_cooldown():
        c=request('player_context')
        return c if any(x['ticks']['value']>0 and x['ticks']['source']['sequence']>before for x in c['item_cooldowns']) else None
    until(received_cooldown,5);current,events=capture('received_positive_cooldown_native_identity')
    notified(current,events,current['item_cooldowns'][0]['ticks'])
    def cleared():
        c=request('player_context');return c if c['item_cooldowns'] and all(x['ticks']['value']==0 for x in c['item_cooldowns']) else None
    until(cleared,8);current,events=capture('received_zero_cooldown_retained')
    notified(current,events,current['item_cooldowns'][0]['ticks'])
    if version=='1.21.11':
        command('item replace entity ClimbingProbe weapon.mainhand with minecraft:ender_pearl[use_cooldown={seconds:1.0,cooldown_group:"voxrig:shared"}] 4')
        request('wait',ms=150);before=request('player_context')['receive_sequence'];request('use_item')
        until(received_cooldown,5);current,events=capture('modern_custom_group_is_not_an_item_registry_id')
        group=next((x for x in current['item_cooldowns'] if x['key']==dict(kind='group',value='voxrig:shared')),None)
        if group is None:raise RuntimeError('custom native cooldown group missing')
        notified(current,events,group['ticks'])
        until(cleared,8);current,events=capture('modern_custom_group_zero_clear')
        group=next(x for x in current['item_cooldowns'] if x['key']['value']=='voxrig:shared')
        notified(current,events,group['ticks'])
    command('experience add ClimbingProbe 1 points');request('wait',ms=150)
    later,events=capture('unrelated_receive_keeps_cooldown_source')
    if later['item_cooldowns']!=current['item_cooldowns']:raise RuntimeError('unrelated packet refreshed cooldown')
    command('difficulty hard')
    until(lambda: (c if (c:=request('player_context'))['difficulty'] and c['difficulty']['value']['id']==3 else None),5)
    current,events=capture('difficulty_context_notification');notified(current,events,current['difficulty'])
    command('gamemode creative ClimbingProbe');request('wait',ms=150)
    command('gamemode survival ClimbingProbe');request('wait',ms=150)
    before=current['session']['world_generation']
    command('execute in minecraft:the_nether run tp ClimbingProbe 0.5 80 0.5 0 0')
    until(lambda: (c if (c:=request('player_context'))['session']['world_generation']!=before else None),10)
    current,events=capture('dimension_respawn_previous_mode_and_cooldown_retirement')
    notified(current,events,current['world_entry'])
    if current['item_cooldowns']:raise RuntimeError('old-world cooldowns survived')
    if current['world_entry']['value']['previous_game_mode']!=1:raise RuntimeError('received previous creative mode missing')
    command('execute in minecraft:overworld run tp ClimbingProbe 0.5 65 0.5 0 0')
    before=current['session']['world_generation']
    until(lambda: (c if (c:=request('player_context'))['session']['world_generation']!=before else None),10)
    current,events=capture('return_dimension_entry_ownership');notified(current,events,current['world_entry'])
    request('prepare',position=[0.5,65.,0.5])
    if version=='1.21.11':
        boundary=verify(version,current,trace)['original_entry']['ordinal']
        def loaded():
            return next((p for p in trace.since(boundary) if p['direction']=='serverbound' and p['phase']=='play' and p['packet_id']==0x2b),None)
        report['checks'][-1]['original_player_loaded']=until(loaded,15)
        request('wait',ms=150)
    report['checks'][-1]['native_kill_result']=command('kill ClimbingProbe')
    until(lambda: (p if (p:=request('player'))['health'] and p['health']['value']['health']==0 else None),5)
    request('respawn')
    before=current['session']['world_generation']
    until(lambda: (c if (c:=request('player_context'))['session']['world_generation']!=before else None),10)
    current,events=capture('same_dimension_death_respawn_keep_data_and_last_death');notified(current,events,current['world_entry'])
    if version=='1.21.11' and current['world_entry']['value']['last_death'] is None:raise RuntimeError('native last death missing')
    trace.expect_disconnect();closed=request('context_disconnect')
    report['checks'].append(dict(name='world_entry_and_cooldowns_readable_after_close',context=closed,original=verify(version,closed,trace)))


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
    sdk=dict(source_revision=revision,binary_build_revision=compiled,binary_sha256=hashlib.sha256(args.binary.read_bytes()).hexdigest(),
             source_diff_sha256=hashlib.sha256(subprocess.check_output(['git','diff','HEAD'],cwd=REPO)).hexdigest(),
             artifacts={f:hashlib.sha256((REPO/f).read_bytes()).hexdigest() for f in ('Cargo.lock','scripts/run_world_entry_context.py','scripts/run_climbing_control.py','scripts/run_common_native.py','scripts/run_player_context.py','examples/climbing_control_probe.rs')})
    for version in args.version or ('1.16.1','1.21.11'):
        run(version,args.binary.resolve(),args.jars.resolve(),check=functools.partial(check,sdk=sdk),server_properties={'max-players':5,'view-distance':3,'simulation-distance':2})
if __name__=='__main__':main()
