// Unchanged native context, replacement, slab placement and fluid-state methods.
// Reuses the existing fail-closed cell provider; no liquid simulation is substituted.
import com.google.gson.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
public final class ExportFluidSlabPlacement {
    static JsonObject fluid(Object state) throws Exception {
        Object value=Class.forName("eog$a").getMethod("y").invoke(state);
        Class<?> fs=Class.forName("flb");
        JsonObject out=new JsonObject();
        boolean empty=(boolean)fs.getMethod("c").invoke(value);
        // Registry ID is retained separately so lava cannot be mistaken for water.
        Object registry=Class.forName("mi").getField("c").get(null);
        Object type=fs.getMethod("a").invoke(value);
        String nativeType=Class.forName("jq").getMethod("b",Object.class).invoke(registry,type).toString();
        out.addProperty("native_type",nativeType);
        out.addProperty("kind",empty?"empty":nativeType.contains("lava")?"lava":"water");
        out.addProperty("source",(boolean)fs.getMethod("b").invoke(value));
        out.addProperty("amount",(int)fs.getMethod("e").invoke(value));
        Map<?,?> entries=(Map<?,?>)Class.forName("eoj").getMethod("G").invoke(value);
        boolean falling=false;
        for(Map.Entry<?,?> entry:entries.entrySet()) {
            String name=(String)Class.forName("epk").getMethod("f").invoke(entry.getKey());
            if(name.equals("falling")) falling=(boolean)entry.getValue();
        }
        out.addProperty("falling",falling);
        return out;
    }
    public static void main(String[] args) throws Exception {
        Class.forName("w").getMethod("a").invoke(null);Class.forName("amv").getMethod("a").invoke(null);
        Object world=ExportSlabPlacement.level(Path.of(args[1]));
        Class<?> carrier=world.getClass().getClassLoader().loadClass("SlabContext");
        Class<?> block=Class.forName("dzq"), state=Class.forName("eoh"), pos=Class.forName("is"), direction=Class.forName("iz"), vec=Class.forName("ftm"), hit=Class.forName("fti"), context=Class.forName("dpu"), stack=Class.forName("dlt");
        Object registry=Class.forName("mi").getField("e").get(null);Map<String,Object> blocks=new TreeMap<>();
        for(Object b:(Iterable<?>)registry)blocks.put(Class.forName("jq").getMethod("b",Object.class).invoke(registry,b).toString(),b);
        Object air=block.getMethod("m").invoke(blocks.get("minecraft:air"));world.getClass().getField("air").set(null,air);
        Object clicked=pos.getConstructor(int.class,int.class,int.class).newInstance(2,2,0),oak=blocks.get("minecraft:oak_slab");
        JsonArray rows=new JsonArray();
        for(String name:List.of("air","stone","oak_slab","spruce_slab","water","lava","bubble_column","kelp")) {
            Object b=blocks.get("minecraft:"+name),definition=block.getMethod("l").invoke(b);
            for(Object bs:(List<?>)Class.forName("eoi").getMethod("a").invoke(definition)) {
                for(Object face:(Object[])direction.getMethod("values").invoke(null))
                for(boolean secondary:new boolean[]{false,true})for(boolean adjacent:new boolean[]{false,true})
                for(float hitY:new float[]{Math.nextDown(.5f),.5f,Math.nextUp(.5f)}) {
                    Map<Object,Object> cells=new HashMap<>();Object neighbor=pos.getMethod("a",direction).invoke(clicked,face);
                    cells.put(clicked,adjacent?block.getMethod("m").invoke(blocks.get("minecraft:stone")):bs);
                    if(adjacent)cells.put(neighbor,bs);world.getClass().getField("cells").set(null,cells);
                    Object held=stack.getConstructor(Class.forName("dwn"),int.class).newInstance(block.getMethod("h").invoke(oak),1);
                    Object point=vec.getConstructor(double.class,double.class,double.class).newInstance(2.25,2.0+(double)hitY,.5);
                    Object result=hit.getConstructor(vec,direction,pos,boolean.class).newInstance(point,face,clicked,false);
                    Field field=carrier.getDeclaredField("secondary");field.setAccessible(true);field.setBoolean(null,secondary);
                    Constructor<?> ctor=carrier.getDeclaredConstructor(Class.forName("dwo"),Class.forName("ddm"),Class.forName("cdb"),stack,hit);ctor.setAccessible(true);
                    Object ctx=ctor.newInstance(world,null,Class.forName("cdb").getField("a").get(null),held,result);
                    Object destination=context.getMethod("a").invoke(ctx),before=world.getClass().getMethod("a_",pos).invoke(world,destination);
                    Object placed=block.getMethod("a",context).invoke(oak,ctx);
                    Map<String,Object> row=new LinkedHashMap<>();
                    row.put("clicked",ExportSlabPlacement.stateJson(cells.get(clicked),adjacent?"minecraft:stone":"minecraft:"+name));
                    row.put("input",ExportSlabPlacement.stateJson(bs,"minecraft:"+name));row.put("input_fluid",fluid(bs));
                    row.put("face",((Enum<?>)face).ordinal());row.put("secondary",secondary);row.put("adjacent",adjacent);row.put("hit_y",hitY);
                    row.put("replace_clicked",context.getMethod("c").invoke(ctx));row.put("can_place",context.getMethod("b").invoke(ctx));
                    row.put("destination",new int[]{(int)pos.getMethod("u").invoke(destination),(int)pos.getMethod("v").invoke(destination),(int)pos.getMethod("w").invoke(destination)});
                    row.put("destination_state",block.getMethod("j",state).invoke(null,before));row.put("before_fluid",fluid(before));
                    row.put("placed_state",ExportSlabPlacement.stateJson(placed,"minecraft:oak_slab"));row.put("placed_fluid",fluid(placed));
                    rows.add(new Gson().toJsonTree(row));
                }
            }
        }
        Files.writeString(Path.of(args[0]),new GsonBuilder().setPrettyPrinting().create().toJson(rows)+"\n");
    }
}
