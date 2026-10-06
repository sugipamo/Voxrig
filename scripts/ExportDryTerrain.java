// Own observer of original registered slab/stair states and native collision/clip.
// No game method body is copied, remapped, patched or replaced; no server starts.
import com.google.gson.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import java.util.stream.Stream;

public final class ExportDryTerrain {
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
            Class<?> slab=cls("caz","ehi"),stair=cls("cbn","ehz"),vector=cls("dem","ftm"),aabb=cls("deg","fth");
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
                Object block=entry.getValue();boolean isSlab=block.getClass()==slab,isStair=block.getClass()==stair;
                if(!isSlab&&!isStair)continue; // Unknown subclasses do not inherit this authority.
                JsonObject physical=new JsonObject();physical.addProperty("name",entry.getKey());physical.addProperty("kind",isSlab?"slab":"stairs");
                float friction=(Float)blockClass.getMethod(old?"j":"g").invoke(block);
                float speed=(Float)blockClass.getMethod(old?"k":"i").invoke(block),jump=(Float)blockClass.getMethod(old?"l":"j").invoke(block);
                if(friction!=.6f||speed!=1f||jump!=1f)throw new IllegalStateException("nondefault terrain physics: "+entry.getKey());
                physical.add("material",ExportStorageOutlines.numbers(friction,speed,jump));blocks.add(physical);
                Object definition=blockClass.getMethod(old?"m":"l").invoke(block);
                for(Object state:(List<?>)cls("cfk","eoi").getMethod("a").invoke(definition)) {
                    JsonObject properties=new JsonObject();Map<String,String> ordered=new TreeMap<>();
                    Map<?,?> entries=(Map<?,?>)cls("cfl","eoj").getMethod(old?"s":"G").invoke(state);
                    for(var e:entries.entrySet())ordered.put((String)cls("cgl","epk").getMethod("f").invoke(e.getKey()),(String)cls("cgl","epk").getMethod(old?"a":"b",Comparable.class).invoke(e.getKey(),e.getValue()));
                    if(!"false".equals(ordered.get("waterlogged")))continue;
                    ordered.forEach(properties::addProperty);
                    Object nativeShape=shape.invoke(state,world,origin,collision);
                    JsonObject row=new JsonObject(),nativeState=new JsonObject();nativeState.addProperty("name",entry.getKey());nativeState.add("properties",properties);
                    row.add("state",nativeState);row.addProperty("native_id",(Integer)blockClass.getMethod(old?"i":"j",cls("cfj","eoh")).invoke(null,state));
                    JsonArray boxes=ExportStorageOutlines.boxes(nativeShape);row.add("collision",boxes);
                    row.add("outline",ExportStorageOutlines.boxes(outline.invoke(state,world,origin,collision)));row.add("auxiliary",ExportStorageOutlines.boxes(aux.invoke(state,world,origin)));
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
                    String clipKey=row.get("outline").toString()+row.get("auxiliary");
                    if(uniqueOutline.putIfAbsent(clipKey,index)==null) {
                        for(int axis=0;axis<3;axis++)for(double offset:new double[]{0,.25,.5,.75,1}) {
                            double[] a={offset,offset,offset},b=a.clone();a[axis]=-2;b[axis]=2;
                            rays.add(ExportStorageOutlines.sample(state,air,index,0,a,b));rays.add(ExportStorageOutlines.sample(state,air,index,0,b,a));
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
