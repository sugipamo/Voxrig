#!/usr/bin/env python3
"""Preserve original packets while delaying one inflight result click until an actual pickup.

No frames are fabricated or replayed. The server owns the item, collect event,
grid consumption and output. Selected and unselected hotbar pickups demonstrate
why a separate operation outcome is a design decision, not a strict-API fix.
"""
import sys,json,threading,time,argparse
from pathlib import Path
import run_climbing_control as h
from run_common_native import PacketTraceProxy,until
p=argparse.ArgumentParser(description="Characterize a real collect during an inflight 1.16.1 crafting take; strict semantics stay unchanged.")
p.add_argument('--accept-eula',action='store_true',required=True)
p.add_argument('--binary',type=Path,required=True)
p.add_argument('--jars',type=Path,required=True)
p.add_argument('--output',type=Path,required=True)
p.add_argument('--case',choices=['selected','unselected','none'],default='selected')
args=p.parse_args()
ROOT=args.output.resolve()
class InflightTake(PacketTraceProxy):
 def __init__(self,*a,**kw):
  super().__init__(*a,**kw);self.armed=False;self.held=[];self.guard=threading.Lock()
 def forward_frame(self,r,d,w):
  if self.armed and r['phase']=='play' and r['direction']=='serverbound' and r['packet_id']==0x09:
   b=bytes.fromhex(r['body_hex'])
   if int.from_bytes(b[1:3],'big',signed=True)==0:
    with self.guard:self.held.append((r,d,w))
    return
  super().forward_frame(r,d,w)
 def release(self):
  self.armed=False
  with self.guard:
   for r,d,w in self.held:super().forward_frame(r,d,w)
   self.held.clear()

def check(version,command,request,trace,report):
 def item(v,name,count):return v and v['value']['kind']=='item' and v['value']['item']['name']=='minecraft:'+name and v['value']['item']['count']==count
 command('tp ClimbingProbe 0.5 65 0.5 0 0');request('prepare',position=[.5,65.,.5]);command('clear ClimbingProbe')
 if args.case=='unselected':command('replaceitem entity ClimbingProbe hotbar.0 minecraft:stone 1')
 command('replaceitem entity ClimbingProbe inventory.0 minecraft:oak_log 1')
 until(lambda:item(request('player')['inventory']['slots'][9],'oak_log',1),5)
 request('inventory_click',slot=9);request('inventory_click',slot=1)
 until(lambda:item(request('crafting')['result'],'oak_planks',4),5)
 before=request('player');mark=trace.mark();trace.armed=True
 sent=request('take_crafting_result');assert sent['send']['dispatched'],sent
 until(lambda:len(trace.held)==1,5)
 if args.case!='none':command('summon minecraft:item 0.5 65 0.5 {Item:{id:"minecraft:oak_log",Count:1b},PickupDelay:0s,Tags:["Mo5Pickup"]}')
 collect=until(lambda:[r for r in trace.since(mark) if r['phase']=='play' and r['direction']=='clientbound' and r['packet_id']==0x55],10) if args.case!='none' else []
 trace.release()
 def done():
  r=request('crafting_take_record');return r if r['stage']!='pending' else None
 complete=until(done,10);time.sleep(.5);complete=request('crafting_take_record')
 player=request('player');native=command('data get entity ClimbingProbe Inventory')
 report['checks'].append(dict(name='pickup_during_dispatched_take',case=args.case,inventory_send=request('inventory_send_record'),changes=request('inventory_changes'),before=before,sent=sent,collect=collect,complete=complete,player=player,native=native,frames=trace.since(mark)))
 print('stage',complete['stage'],'reason',complete['requires_inspection'],'native',native,flush=True)
 assert complete['stage']==('requires_inspection' if args.case=='selected' else 'observed_taken'),complete
 if args.case=='selected':assert 'equipped hand changed' in complete['requires_inspection'],complete
 assert item(player['inventory']['cursor'],'oak_planks',4),player
 if args.case!='none':assert item(player['inventory']['slots'][36 if args.case=='selected' else 37],'oak_log',1),player
 assert player['inventory']['slots'][1]['value']['kind']=='empty',player
 trace.expect_disconnect();request('disconnect')
h.ROOT=ROOT
h.run('1.16.1',args.binary.resolve(),args.jars.resolve(),check=check,trace_factory=InflightTake)
