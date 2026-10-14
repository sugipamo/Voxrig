#!/usr/bin/env python3
"""Execute unchanged original Player/LivingEntity glide in an explicit client-branch numerical origin."""
import argparse,gzip,hashlib,json,re,sys
from pathlib import Path
from movement_oracle import run as original
REPO=Path(__file__).resolve().parents[1]
def sha(p):return hashlib.sha256(p.read_bytes()).hexdigest()
def once(s,a,b):assert s.count(a)==1,a;return s.replace(a,b)
def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('--downloads',type=Path,required=True);p.add_argument('--work',type=Path,required=True);p.add_argument('--scenarios',type=Path,default=REPO/'scripts/movement_oracle/glide_scenarios.json');p.add_argument('--output',type=Path,default=REPO/'data/client_api/glide_oracle.json.gz');a=p.parse_args();generated=a.work.resolve()/'generated_owned_harness';generated.mkdir(parents=True,exist_ok=True);sources={}
 for v,(_,_,directory) in original.VERSIONS.items():
  base=REPO/'scripts/movement_oracle'/directory/'MovementOracle.java';text=base.read_text();mappings=(a.downloads/f'{v}-server-mappings.txt').read_text();match=re.search(r'^net.minecraft.world.level.Level -> [^\n]+:\n(.*?)(?=^[A-Za-z])',mappings,re.M|re.S);assert match
  field=re.search(r'^    boolean isClientSide -> (\S+)$',match[1],re.M);assert field
  helper='''    static void ownedClientBranch(Level level, boolean value) {
        try { java.lang.reflect.Field field = Level.class.getDeclaredField("FIELD"); field.setAccessible(true); field.setBoolean(level,value); }
        catch(Exception error) { throw new RuntimeException(error); }
    }
'''.replace('FIELD',field[1])
  text=once(text,'    static final int CLEAR = 12;',helper+'    static final int CLEAR = 12;')
  extra='''        player.setItemSlot(net.minecraft.world.entity.EquipmentSlot.CHEST, new net.minecraft.world.item.ItemStack(net.minecraft.world.item.Items.ELYTRA));
        if (scenario.has("velocity")) { JsonArray v=scenario.getAsJsonArray("velocity"); player.setDeltaMovement(v.get(0).getAsDouble(),v.get(1).getAsDouble(),v.get(2).getAsDouble()); }
        if(!player.tryToStartFallFlying()) throw new IllegalStateException("original glide start refused");
        player.setPose(Pose.FALL_FLYING); player.refreshDimensions();
'''
  text=once(text,'        JsonObject initial = new JsonObject();',extra+'        JsonObject initial = new JsonObject();\n        initial.addProperty("gliding",player.isFallFlying());')
  text=once(text,'            player.tick();','''            ownedClientBranch(level,true);
            try { player.tick(); } finally { ownedClientBranch(level,false); }''')
  needle='            f.addProperty("speed", Float.toString(player.getSpeed()));'
  text=once(text,needle,needle+'''\n            f.addProperty("gliding",player.isFallFlying());
            f.addProperty("eye_height",Float.toString(player.getEyeHeight()));
            AABB box=player.getBoundingBox();
            f.add("bounds",vec(box.minX-origin.getX(),box.minY-origin.getY(),box.minZ-origin.getZ()));
            f.add("bounds_max",vec(box.maxX-origin.getX(),box.maxY-origin.getY(),box.maxZ-origin.getZ()));''')
  folder=generated/directory;folder.mkdir(exist_ok=True);target=folder/'MovementOracle.java';target.write_text(text);sources[v]=dict(base_harness_sha256=sha(base),generated_harness_sha256=sha(target),client_branch_field=field[1])
 original.HERE=generated;arguments=sys.argv
 try:
  sys.argv=[str(original.__file__),'--downloads',str(a.downloads.resolve()),'--work',str(a.work.resolve()/'native'),'--scenarios',str(a.scenarios.resolve()),'--output',str(a.output.resolve()),'--blocks-output',str(a.work.resolve()/'original_blocks')];original.main()
 finally:sys.argv=arguments
 f=json.loads(gzip.decompress(a.output.read_bytes()));f['authority']='Unchanged official Player/LivingEntity tick/travel/pose on original registered server terrain, with the original Level client-side branch flag explicitly set during each owned numerical tick and restored afterward. Owned LocalPlayer input port remains unchanged; this is not a real ClientLevel packet clock or server acceptance.';f['wrapper_sha256']=sha(Path(__file__));f['owned_sources']=sources;a.output.write_bytes(gzip.compress((json.dumps(f,separators=(',',':'))+'\n').encode(),mtime=0))
if __name__=='__main__':main()
