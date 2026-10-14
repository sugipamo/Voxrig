#!/usr/bin/env python3
"""Execute original packet handlers and ClientLevel entity tick dispatch.

Only our harness source, numeric results and verified input hashes are published.
Rendering constructors are skipped; world registration is original, AI/travel
is isolated, and membership/packet/tick boundaries are explicit harness stages.
This does not establish an autonomous native client clock or passenger physics.
"""
import sys
sys.path.insert(0,str(__import__("pathlib").Path(__file__).resolve().parents[0]))
import argparse
import gzip
import json
import os
from pathlib import Path
from run_entity_pose_oracle import mappings, member
from movement_oracle.run import classpath, digest, run, VERSIONS, SPECIAL_SOURCE_SHA256, PROPERTIES
REPO=Path(__file__).resolve().parents[1]
CLIENT={'1.16.1':('c9abbe8ee4fa490751ca70635340b7cf00db83ff','ddf517a4f6750f4c15189de4e03246ae1f916cf5'),'1.21.11':('ba2df812c2d12e0219c489c4cd9a5e1f0760f5bd','031a68bebf55d824f66d6573d8c752f0e1bf232a')}

def main():
    p=argparse.ArgumentParser(description=__doc__)
    p.add_argument('--downloads',type=Path,required=True)
    p.add_argument('--client-downloads',type=Path,required=True)
    p.add_argument('--work',type=Path,required=True)
    p.add_argument('--output',type=Path,default=REPO/'data/client_api/entity_owner_oracle.json.gz')
    args=p.parse_args();args.work.mkdir(parents=True,exist_ok=True)
    tool=(args.downloads/'SpecialSource-1.11.4-shaded.jar').resolve()
    assert digest(tool.read_bytes())==SPECIAL_SOURCE_SHA256
    result={'schema':1,'world':{'floor_y':64,'floor_region':[-16,-16,32,16],'floor_state':'minecraft:stone'},'authority':'Unchanged official client packet handlers, original AI-step/interpolation/push, original Player.aiStep neutral travel and mounted Boat.tick in real ClientLevel runtime dispatch. Headless allocation omits graphics constructors/audio delivery; ClientLevel reads original registered chunks, light and environment attributes. Modern section visibility and own player mode/input guards are explicit. Passenger rideTick and graphics LocalPlayer input preprocessing are isolated. These are explicit owned numerical ticks, not a native clock, full NPC baseTick/fluid/AI, server-current motion or acceptance.','versions':{}}
    for port,(version,(sha,msha,_)) in enumerate(list(VERSIONS.items()),start=25760):
        work=(args.work/version).resolve();work.mkdir(parents=True,exist_ok=True);(work/'classes').mkdir(exist_ok=True)
        jar=(args.downloads/f'{version}-server.jar').resolve();mapping=(args.downloads/f'{version}-server-mappings.txt').resolve()
        assert digest(jar.read_bytes(),'sha1')==sha and digest(mapping.read_bytes(),'sha1')==msha
        game,cp=classpath(version,jar,work)
        legacy=version=='1.16.1';game=(args.client_downloads/f'{version}-client.jar').resolve();mapping=(args.client_downloads/f'{version}-client-mappings.txt').resolve();cp=[game,*cp] if legacy else [game,*cp[1:]]
        cm=mappings(mapping)
        if legacy:
            extra=[member(cm,'net.minecraft.client.multiplayer.ClientLevel',' '+n+' ->') for n in ['entitiesById','players','chunkSource']]+[member(cm,'net.minecraft.client.Minecraft',' gameThread ->')]+[member(cm,'net.minecraft.client.multiplayer.ClientPacketListener',' '+n+' ->') for n in ['minecraft','level']]+[member(cm,'net.minecraft.world.level.Level',' isClientSide ->')]+[member(cm,'net.minecraft.client.multiplayer.ClientLevel',' '+n+' ->') for n in ['scoreboard','connection']]
        else:
            extra=[member(cm,'net.minecraft.client.multiplayer.ClientLevel',' '+n+' ->') for n in ['tickingEntities','tickRateManager']]+[member(cm,'net.minecraft.client.Minecraft',' packetProcessor ->'),member(cm,'net.minecraft.client.multiplayer.ClientCommonPacketListenerImpl',' minecraft ->'),member(cm,'net.minecraft.client.multiplayer.ClientPacketListener',' level ->'),member(cm,'net.minecraft.world.level.Level',' isClientSide ->'),member(cm,'net.minecraft.world.entity.InterpolationHandler',' interpolationData ->'),member(cm,'net.minecraft.world.entity.InterpolationHandler$InterpolationData',' steps ->'),cm['net.minecraft.world.entity.InterpolationHandler$InterpolationData']['class'].split('$')[-1]]+[member(cm,'net.minecraft.client.multiplayer.ClientLevel',' '+n+' ->') for n in ['chunkSource','entityStorage','connection']]+[member(cm,'net.minecraft.client.multiplayer.ClientPacketListener',' scoreboard ->'),member(cm,'net.minecraft.client.multiplayer.ClientLevel',' minecraft ->'),member(cm,'net.minecraft.client.Minecraft',' player ->')]+[member(cm,'net.minecraft.client.player.LocalPlayer',' '+n+' ->') for n in ['minecraft','connection']]+[member(cm,'net.minecraft.client.multiplayer.ClientLevel',' '+n+' ->') for n in ['dragonParts','players','worldBorder']]+[member(cm,'net.minecraft.world.entity.player.Player','void aiStep() ->')]
        if legacy:
            extra += [member(cm,'net.minecraft.world.entity.player.Player','void aiStep() ->')]+[member(cm,'net.minecraft.client.player.LocalPlayer',' '+n+' ->') for n in ['minecraft','connection']]+[member(cm,'net.minecraft.client.Minecraft',' player ->'),member(cm,'net.minecraft.client.multiplayer.ClientLevel',' clientLevelData ->'),member(cm,'net.minecraft.world.entity.LivingEntity',' lerpSteps ->'),member(cm,'net.minecraft.network.syncher.SynchedEntityData',' entity ->'),member(cm,'net.minecraft.client.multiplayer.ClientLevel',' minecraft ->')]
        else:
            extra += [member(cm,'net.minecraft.client.player.AbstractClientPlayer',' clientAvatarState ->'),member(cm,'net.minecraft.network.syncher.SynchedEntityData',' entity ->'),member(cm,'net.minecraft.client.multiplayer.ClientLevel',' environmentAttributes ->')]
        extra += [member(cm,'net.minecraft.world.entity.LivingEntity',' SLEEPING_POS_ID ->')]
        extra += [member(cm,'net.minecraft.world.entity.'+('AgableMob' if legacy else 'AgeableMob'),' DATA_BABY_ID ->')]
        assert digest(game.read_bytes(),'sha1')==CLIENT[version][0]
        assert digest(mapping.read_bytes(),'sha1')==CLIENT[version][1]
        named=work/'named.jar'
        run(['java','-jar',str(tool),'--in-jar',str(game),'--out-jar',str(named),'--srg-in',str(mapping)],work,work/'remap.log')
        source=REPO/'scripts/entity_owner_oracle'/version/'EntityTickOracle.java'
        run(['javac','-nowarn','--release','21','-d',str(work/'classes'),'-cp',os.pathsep.join(str(p) for p in [named,*cp[1:]]),str(source)],work,work/'javac.log')
        run(['jar','cf',str(work/'harness-named.jar'),'-C',str(work/'classes'),'voxrig'],work,work/'jar.log')
        run(['java','-cp',f'{tool}:{named}','net.md_5.specialsource.SpecialSource','--live','--in-jar',str(work/'harness-named.jar'),'--out-jar',str(work/'harness.jar'),'--srg-in',str(mapping),'--reverse'],work,work/'unmap.log')
        server=work/'server';server.mkdir(exist_ok=True);(server/'eula.txt').write_text('eula=true\n')
        (server/'server.properties').write_text(PROPERTIES.format(port=port,level_type='flat' if legacy else 'minecraft\\:flat'))
        output=work/'original-results.json'
        run(['java','-Xmx1G','--add-opens','java.base/java.lang=ALL-UNNAMED','-cp',os.pathsep.join(str(p) for p in [*cp,work/'harness.jar']),'voxrig.oracle.EntityTickOracle',str(output),*extra],server,work/'server.log')
        rows=json.loads(output.read_text());assert len(rows)==48;print(version,sum(len(r["steps"]) for r in rows),"stages",flush=True)
        result['versions'][version]=dict(original_server_jar_sha1=sha,server_mappings_sha1=msha,original_client_jar_sha1=CLIENT[version][0],client_mappings_sha1=CLIENT[version][1],original_output_sha256=digest(output.read_bytes()),harness_sha256=digest(source.read_bytes()),results=rows)
    result['runner_sha256']=digest(Path(__file__).read_bytes())
    args.output.write_bytes(gzip.compress((json.dumps(result,separators=(',', ':'))+'\n').encode(),mtime=0))
    print('original entity packet/tick schedules:',sum(len(v['results']) for v in result['versions'].values()))
if __name__=='__main__':main()
