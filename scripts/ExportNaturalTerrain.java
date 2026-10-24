// Own observer of original registered natural terrain states and native collision/clip.
// No game method body is copied, remapped, patched or replaced; no server starts.
import com.google.gson.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import java.util.stream.Stream;

public final class ExportNaturalTerrain {
    static Class<?> cls(String oldName,String newName) throws Exception {
        return ExportStorageOutlines.cls(oldName,newName);
    }
    static JsonArray vec(Object value,boolean old) throws Exception {
        String fields=old?"bcd":"ghi";JsonArray out=new JsonArray();
        for(int i=0;i<3;i++)out.add(value.getClass().getField(fields.substring(i,i+1)).getDouble(value));
        return out;
    }
    public static void main(String[] args) throws Exception {
        boolean old=args[0].equals("1.16.1");
        if(!old&&!args[0].equals("1.21.11"))throw new IllegalArgumentException("version");
        ExportStorageOutlines.legacy=old;
        if(!old)Class.forName("w").getMethod("a").invoke(null);
        Object version=cls("u","w").getMethod(old?"a":"b").invoke(null);
        if(!cls("com.mojang.bridge.game.GameVersion","aa").getMethod(old?"getName":"c").invoke(version).equals(args[0]))throw new IllegalStateException("version mismatch");
        cls("uj","amv").getMethod("a").invoke(null);
        try {
            Class<?> registryClass=cls("gl","jq"),blockClass=cls("bvr","dzq"),stateBase=cls("cfi$a","eog$a");
            Class<?> view=cls("bpg","dvt"),pos=cls("fu","is"),ctx=cls("der","ftr");
            Class<?> vector=cls("dem","ftm"),aabb=cls("deg","fth");
            Object registry=cls("gl","mi").getField(old?"aj":"e").get(null);
            Method key=registryClass.getMethod("b",Object.class);
            Map<String,Object> registered=new TreeMap<>();
            for(Object block:(Iterable<?>)registry)registered.put(key.invoke(registry,block).toString(),block);
            Object air=blockClass.getMethod(old?"n":"m").invoke(registered.get("minecraft:air"));
            Object world=cls("bpp","dwf").getField("a").get(null),origin=pos.getField(old?"b":"c").get(null),collision=ctx.getMethod("a").invoke(null);
            Method shape=stateBase.getMethod("b",view,pos,ctx),outline=stateBase.getMethod("a",view,pos,ctx),aux=stateBase.getMethod(old?"m":"i",view,pos);
            Method collide=cls("aom","cgk").getDeclaredMethod("a",vector,aabb,old?Class.forName("aee"):List.class);collide.setAccessible(true);
            Class<?> shapes=cls("dfd","fud"),voxel=cls("dfg","fug"),booleanOp=cls("deq","ftq");
            Method boxShape=shapes.getMethod("a",aabb),intersects=shapes.getMethod("c",voxel,voxel,booleanOp);
            Object and=booleanOp.getField("i").get(null);
            JsonArray blocks=new JsonArray(),states=new JsonArray(),collisions=new JsonArray(),rays=new JsonArray();
            Map<String,Integer> uniqueCollision=new TreeMap<>(),uniqueOutline=new TreeMap<>();
            for(var entry:registered.entrySet()) {
                Object block=entry.getValue();
                String name=entry.getKey().substring("minecraft:".length());
                // Explicit families of unchanged official registry entries. Only
                // collision-independent plants and ordinary dry surfaces; no
                // berry/wither-rose/cactus/powder-snow damage or movement effects.
                boolean family=name.endsWith("_log")||name.endsWith("_wood")||name.endsWith("_stem")
                    ||name.endsWith("_hyphae")||name.endsWith("_leaves")||name.endsWith("_sapling")
                    ||name.endsWith("_ore");
                boolean named=Set.of(
                    "stone","granite","polished_granite","diorite","polished_diorite","andesite","polished_andesite",
                    "deepslate","cobbled_deepslate","tuff","calcite","dripstone_block",
                    "dirt","coarse_dirt","rooted_dirt","grass_block","podzol","mycelium","dirt_path","grass_path","farmland",
                    "clay","sand","red_sand","gravel","sandstone","red_sandstone","snow","snow_block",
                    "netherrack","basalt","smooth_basalt","polished_basalt","blackstone","end_stone",
                    "moss_block","pale_moss_block","moss_carpet","mud","packed_mud","muddy_mangrove_roots",
                    "oak_planks","spruce_planks","birch_planks","jungle_planks","acacia_planks","dark_oak_planks",
                    "mangrove_planks","cherry_planks","pale_oak_planks","crimson_planks","warped_planks",
                    "grass","short_grass","tall_grass","fern","large_fern","dead_bush",
                    "dandelion","poppy","blue_orchid","allium","azure_bluet","red_tulip","orange_tulip",
                    "white_tulip","pink_tulip","oxeye_daisy","cornflower","lily_of_the_valley",
                    "sunflower","lilac","rose_bush","peony","brown_mushroom","red_mushroom",
                    "crimson_fungus","warped_fungus","crimson_roots","warped_roots","nether_sprouts",
                    "azalea","flowering_azalea","bush","firefly_bush","wildflowers","leaf_litter","pink_petals",
                    "short_dry_grass","tall_dry_grass","torchflower","torchflower_crop","pitcher_plant",
                    "wheat","carrots","potatoes","beetroots","pumpkin","carved_pumpkin","melon",
                    "brown_mushroom_block","red_mushroom_block","mushroom_stem"
                ).contains(name);
                // stems with special shapes/neighbor hooks are outside the log family.
                if((!family&&!named)||name.equals("pumpkin_stem")||name.equals("melon_stem")
                    ||name.startsWith("attached_")||name.equals("mangrove_propagule"))continue;
                // Exact original registry entries; no unregistered/modded subclass authority.
                JsonObject physical=new JsonObject();physical.addProperty("name",entry.getKey());physical.addProperty("kind","natural");physical.addProperty("native_class",block.getClass().getName());
                float friction=(Float)blockClass.getMethod(old?"j":"g").invoke(block);
                float speed=(Float)blockClass.getMethod(old?"k":"i").invoke(block),jump=(Float)blockClass.getMethod(old?"l":"j").invoke(block);
                if(friction!=.6f||speed!=1f||jump!=1f)throw new IllegalStateException("nondefault terrain physics: "+entry.getKey());
                physical.add("material",ExportStorageOutlines.numbers(friction,speed,jump));blocks.add(physical);
                Object definition=blockClass.getMethod(old?"m":"l").invoke(block);
                for(Object state:(List<?>)cls("cfk","eoi").getMethod("a").invoke(definition)) {
                    JsonObject properties=new JsonObject();Map<String,String> ordered=new TreeMap<>();
                    Map<?,?> entries=(Map<?,?>)cls("cfl","eoj").getMethod(old?"s":"G").invoke(state);
                    for(var e:entries.entrySet())ordered.put((String)cls("cgl","epk").getMethod("f").invoke(e.getKey()),(String)cls("cgl","epk").getMethod(old?"a":"b",Comparable.class).invoke(e.getKey(),e.getValue()));
                    if(ordered.containsKey("waterlogged")&&!"false".equals(ordered.get("waterlogged")))continue;
                    ordered.forEach(properties::addProperty);
                    Object nativeShape=shape.invoke(state,world,origin,collision);
                    JsonObject row=new JsonObject(),nativeState=new JsonObject();nativeState.addProperty("name",entry.getKey());nativeState.add("properties",properties);
                    row.add("state",nativeState);row.addProperty("native_id",(Integer)blockClass.getMethod(old?"i":"j",cls("cfj","eoh")).invoke(null,state));
                    JsonArray boxes=ExportStorageOutlines.boxes(nativeShape);row.add("collision",boxes);
                    row.add("outline",new JsonArray());row.add("auxiliary",new JsonArray());
                    int index=states.size();states.add(row);
                    if(uniqueCollision.putIfAbsent(boxes.toString(),index)==null) {
                        // The original combined VoxelShape is used, not recreated
                        // Rust boxes or an implementation-generated expected value.
                        for(double[] p:List.of(new double[]{.5,1,.5},new double[]{.5,.5,.5},new double[]{-.3,.5,.5},new double[]{.5,.5,-.3},new double[]{.5,1.2,.5})) {
                            double half=(double)(.6f/2f),height=(double)1.8f;
                            Object bounds=aabb.getConstructor(double.class,double.class,double.class,double.class,double.class,double.class).newInstance(p[0]-half,p[1],p[2]-half,p[0]+half,p[1]+height,p[2]+half);
                            boolean initialIntersection=(Boolean)intersects.invoke(null,nativeShape,boxShape.invoke(null,bounds),and);
                            for(double[] d:List.of(new double[]{.4,-.08,.2},new double[]{.2,-.08,.4},new double[]{0,-.4,0},new double[]{0,.42,0},new double[]{-.4,0,-.4},new double[]{1e-8,-1e-8,0})) {
                                Object movement=vector.getConstructor(double.class,double.class,double.class).newInstance(d[0],d[1],d[2]);
                                Object colliders=old?Class.forName("aee").getConstructor(Stream.class).newInstance(Stream.of(nativeShape)):List.of(nativeShape);
                                Object actual=collide.invoke(null,movement,bounds,colliders);
                                JsonObject sample=new JsonObject();sample.addProperty("state_index",index);sample.addProperty("initial_intersection",initialIntersection);sample.add("position",ExportStorageOutlines.numbers(p));sample.add("motion",ExportStorageOutlines.numbers(d));sample.add("expected",vec(actual,old));collisions.add(sample);
                            }
                        }
                    }

                }
            }
            JsonObject output=new JsonObject();output.addProperty("version",args[0]);output.add("blocks",blocks);output.add("states",states);output.add("collisions",collisions);output.add("rays",rays);
            output.addProperty("unique_collision_shapes",uniqueCollision.size());output.addProperty("unique_clip_shapes",uniqueOutline.size());
            Files.writeString(Path.of(args[1]),new GsonBuilder().serializeNulls().create().toJson(output)+"\n");
            System.out.println(args[0]+": "+blocks.size()+" blocks, "+states.size()+" dry states, "+collisions.size()+" original collisions, "+rays.size()+" clips");
        } finally {if(old)Class.forName("v").getMethod("h").invoke(null);}
    }
}
