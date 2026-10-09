#!/usr/bin/env python3
"""Common map snapshots checked against original official map packets."""
import argparse
from collections import Counter
import functools
import gzip
import hashlib
import json
from pathlib import Path
import re
import subprocess

from run_climbing_control import REPO, run
from run_common_native import NativeSocialReader, PacketTraceProxy, until
from run_chunk_context import nbt, reader


def map_trace(port, version, path):
    return PacketTraceProxy(port,version,path,body_capture_filter=lambda frame:
        frame["direction"]=="clientbound" and frame["phase"]=="play" and
        frame["packet_id"]==(0x26 if version=="1.16.1" else 0x31) and frame["body_length"]<=1_130_496)


def decode(version, frame):
    modern = version!="1.16.1"
    r = NativeSocialReader(frame,version)
    result = dict(id=r.integer(),scale=int.from_bytes(r.take(1),'big',signed=True))
    result['tracking_position'] = None if modern else r.boolean()
    result['locked'] = r.boolean()
    result['icons'] = None
    if not modern or r.boolean():
        icons=[]
        for _ in range(r.integer()):
            kind = r.integer()
            icon=dict(kind={"ModernRegistryId" if modern else "LegacyOrdinal":kind},
                      x=int.from_bytes(r.take(1),'big',signed=True),y=int.from_bytes(r.take(1),'big',signed=True),
                      encoded_rotation=r.take(1)[0],name=None)
            if r.boolean():
                if modern:
                    start=r.cursor; tag=r.take(1)[0]; r.nbt(tag)
                    icon['name']=dict(kind='native_nbt',bytes=list(r.raw[start:r.cursor]))
                else:
                    icon['name']=dict(kind='legacy_json',json=r.string())
            icons.append(icon)
        result['icons']=icons
    width=r.take(1)[0]
    result['patch']=None
    if width:
        height,x,y=r.take(3)
        colors=list(r.take(r.integer()))
        if len(colors)!=width*height or x+width>128 or y+height>128:
            raise RuntimeError('invalid original map rectangle')
        result['patch']=dict(width=width,height=height,x=x,y=y,colors=colors)
    r.end()
    return result


def digest(value):
    return hashlib.sha256(json.dumps(value,separators=(',',':'),sort_keys=True).encode()).hexdigest()


def verify(version, response, trace):
    context=response['context']
    if context is None:
        raise RuntimeError('expected received map missing')
    peers=[p for p in trace.records if p['direction']=='clientbound' and p['phase'] in ('configuration','play')]
    expected=[None]*16384
    fields={}; originals=[]; previous=0
    for sequence,frame in enumerate(peers,1):
        if not context['map']['first_sequence']<=sequence<=context['receive_sequence']:
            continue
        if frame['phase']!='play' or frame['packet_id']!=(0x26 if version=='1.16.1' else 0x31):
            continue
        value=decode(version,frame)
        if value['id']!=context['map']['native_id']:
            continue
        previous=sequence
        receipt=lambda value:dict(value=value,source=dict(kind='received',sequence=sequence))
        fields.update(scale=receipt(value['scale']),locked=receipt(value['locked']))
        fields['tracking_position']=None if value['tracking_position'] is None else receipt(value['tracking_position'])
        if value['icons'] is not None:
            fields['icons']=receipt(value['icons'])
        patch=value['patch']
        if patch is not None:
            for row in range(patch['height']):
                for column in range(patch['width']):
                    expected[(patch['y']+row)*128+patch['x']+column]=[patch['colors'][row*patch['width']+column],sequence]
        originals.append(dict(receive_sequence=sequence,packet=frame,
            icon_list_provided=value['icons'] is not None,
            patch=None if patch is None else {k:v for k,v in patch.items() if k!='colors'}))
    fields.setdefault('icons',None)
    if not previous or any(context[k]!=v for k,v in fields.items()):
        raise RuntimeError('common map field differs from original packet/source')
    if response['pixels']!=expected:
        raise RuntimeError('map pixels/coverage/original sources differ from replayed original rectangles')
    return dict(context=context,known_pixels=sum(v is not None for v in expected),
        pixels_and_sources_sha256=digest(expected),sources=dict(Counter(v[1] for v in expected if v is not None)),
        originals=originals)


def check(version, command, request, trace, report, *, sdk):
    report['sdk']=sdk
    if request('map_context',id=0)['context'] is not None:
        raise RuntimeError('unreceived map was fabricated')
    report['checks'].append(dict(name='unreceived_map_is_missing',native_id=0))
    command('give ClimbingProbe minecraft:map 1')
    def held(kind):
        value=request('player')['inventory']['slots'][36]
        return value is not None and value['value']['kind']==kind
    until(lambda: held('item'),10)
    request('use_item')
    native=until(lambda: (value if 'filled_map' in (value:=command('data get entity ClimbingProbe Inventory')) else None),10)
    match=re.search(r'(?:map:|"?minecraft:map_id"?:)\s*(\d+)',native)
    if match is None:
        raise RuntimeError('actual filled-map native ID missing: '+native)
    identifier=int(match[1])
    def give_map():
        return command('give ClimbingProbe '+('minecraft:filled_map{map:'+str(identifier)+'}' if version=='1.16.1' else
                       'minecraft:filled_map[minecraft:map_id='+str(identifier)+']')+' 1')
    first=until(lambda: (value if (value:=request('map_context',id=identifier))['context'] is not None else None),20)
    request('map_context',id=identifier,save=True)
    saved=request('saved_map_context')
    report['checks'].append(dict(name='first_received_map',native_item=native,**verify(version,first,trace)))
    report['checks'].append(dict(name='saved_map_keeps_original_receipts',**verify(version,saved,trace)))
    boundary=first['context']['receive_sequence']
    command('fill -16 64 -16 16 64 16 minecraft:gold_block')
    def changed():
        value=request('map_context',id=identifier)
        return value if any(p is not None and p[1]>boundary and p[0]!=(first['pixels'][i][0] if first['pixels'][i] is not None else None)
                            for i,p in enumerate(value['pixels'])) else None
    changed=until(changed,30)
    if changed['context']['map']!=first['context']['map']:
        raise RuntimeError('partial map update replaced original identity')
    report['checks'].append(dict(name='changed_pixels_keep_unmodified_cell_sources',**verify(version,changed,trace)))
    # Stop map sampling through the normal item lifecycle before saving. This
    # gives an independent stable native canvas while in-flight receipts drain.
    command('clear ClimbingProbe minecraft:filled_map')
    until(lambda: held('empty'),10)
    command('save-all flush')
    candidates=list((Path(trace.log.name).parent/'world/data').rglob('map_'+str(identifier)+'.dat'))
    if len(candidates)!=1:
        raise RuntimeError('original saved map file missing or ambiguous')
    saved_file=candidates[0]
    encoded=saved_file.read_bytes()
    raw=gzip.decompress(encoded)
    root,_=nbt(reader(raw,version),True)
    native_colors=[v & 255 for v in root['data'][1]['colors'][1]]
    if len(native_colors)!=16384:
        raise RuntimeError('original saved map colors have wrong geometry')
    def matches_native():
        value=request('map_context',id=identifier)
        return value if all(p is None or p[0]==native_colors[i] for i,p in enumerate(value['pixels'])) else None
    current=until(matches_native,15)
    report['checks'].append(dict(name='received_pixels_match_independently_saved_official_map',
        native_file_sha256=hashlib.sha256(encoded).hexdigest(),native_colors_sha256=digest(native_colors),
        native_colors=native_colors,**verify(version,current,trace)))
    give_map()
    if request('saved_map_context')!=saved:
        raise RuntimeError('later partial patch modified saved map')
    command('tp ClimbingProbe 6.5 65 6.5 90 0')
    until(lambda: (value if (value:=request('map_context',id=identifier))['context']['icons'] is not None
                  and value['context']['icons']['source']['sequence']>current['context']['receive_sequence']
                  and value['context']['icons']['value']!=current['context']['icons']['value'] else None),15)
    moved=request('map_context',id=identifier)
    report['checks'].append(dict(name='fresh_icons_keep_pixel_sources',**verify(version,moved,trace)))
    generation=moved['context']['map']['session']['world_generation']
    command('clear ClimbingProbe minecraft:filled_map')
    command('execute in minecraft:the_nether run tp ClimbingProbe 0.5 80 0.5 0 0')
    until(lambda: request('player')['session']['world_generation']!=generation,20)
    if request('map_context',id=identifier)['context'] is not None:
        raise RuntimeError('map cache crossed world boundary without a new receipt')
    if request('saved_map_context')!=saved:
        raise RuntimeError('world replacement reinterpreted old map')
    report['checks'].append(dict(name='world_change_retires_live_map_and_preserves_saved',saved=verify(version,saved,trace)))
    # The same native ID can be held in another world. New context must start
    # with new receipt identity/coverage, without resurrecting old pixel sources.
    give_map()
    renewed=until(lambda: (value if (value:=request('map_context',id=identifier))['context'] is not None else None),20)
    if renewed['context']['map']==first['context']['map']:
        raise RuntimeError('native map ID reuse resurrected old observation identity')
    report['checks'].append(dict(name='same_native_map_id_new_world_receipt',**verify(version,renewed,trace)))
    trace.expect_disconnect()
    closed=request('map_context_disconnect',id=identifier)
    if closed['context']['map']!=renewed['context']['map'] or closed['saved']!=saved:
        raise RuntimeError('map reads after closure lost original facts')
    report['checks'].append(dict(name='maps_readable_after_close',**verify(version,closed,trace)))


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
        raise RuntimeError('declared built SDK differs from current runtime source')
    sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
    sdk=dict(source_revision=revision,binary_build_revision=compiled,
             source_diff_sha256=hashlib.sha256(subprocess.check_output(['git','diff','HEAD'],cwd=REPO)).hexdigest(),
             binary_sha256=sha(args.binary),artifacts={p:sha(REPO/p) for p in (
                 'Cargo.lock','scripts/run_map_context.py','scripts/run_climbing_control.py',
                 'scripts/run_common_native.py','examples/climbing_control_probe.rs')})
    for version in args.version or ('1.16.1','1.21.11'):
        run(version,args.binary.resolve(),args.jars.resolve(),check=functools.partial(check,sdk=sdk),trace_factory=map_trace)


if __name__=='__main__':
    main()
