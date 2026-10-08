// Own reflection/traversal driver for unchanged original crafting-table shapes.
// No world/server starts and no native method body is copied or replaced.
import com.google.gson.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;

public final class ExportCraftingOutlines {
    static Method inherited(Class<?> owner,String name,Class<?>... parameters) throws Exception {
        for(Class<?> type=owner;type!=null;type=type.getSuperclass()) {
            try {Method method=type.getDeclaredMethod(name,parameters);method.setAccessible(true);return method;}
            catch(NoSuchMethodException absent) {}
        }
        throw new NoSuchMethodException(owner.getName()+"."+name);
    }
    public static void main(String[] args) throws Exception {
        boolean legacy=args[0].equals("1.16.1");
        if(!legacy&&!args[0].equals("1.21.11"))throw new IllegalArgumentException("version");
        ExportStorageOutlines.legacy=legacy;
        if(!legacy)Class.forName("w").getMethod("a").invoke(null);
        Object version=ExportStorageOutlines.cls("u","w").getMethod(legacy?"a":"b").invoke(null);
        String actual=(String)ExportStorageOutlines.cls("com.mojang.bridge.game.GameVersion","aa")
            .getMethod(legacy?"getName":"c").invoke(version);
        if(!actual.equals(args[0]))throw new IllegalStateException("version mismatch");
        ExportStorageOutlines.cls("uj","amv").getMethod("a").invoke(null);
        try {
            Class<?> registryClass=ExportStorageOutlines.cls("gl","jq"),blockClass=ExportStorageOutlines.cls("bvr","dzq");
            Class<?> stateClass=ExportStorageOutlines.cls("cfj","eoh"),viewClass=ExportStorageOutlines.cls("bpg","dvt");
            Class<?> positionClass=ExportStorageOutlines.cls("fu","is"),collisionClass=ExportStorageOutlines.cls("der","ftr");
            Object registry=ExportStorageOutlines.cls("gl","mi").getField(legacy?"aj":"e").get(null);
            Method name=registryClass.getMethod("b",Object.class);
            Map<String,Object> blocks=new TreeMap<>();
            for(Object block:(Iterable<?>)registry)blocks.put(name.invoke(registry,block).toString(),block);
            Object block=Objects.requireNonNull(blocks.get("minecraft:crafting_table"));
            Object air=blockClass.getMethod(legacy?"n":"m").invoke(blocks.get("minecraft:air"));
            Object world=ExportStorageOutlines.cls("bpp","dwf").getField("a").get(null);
            Object origin=positionClass.getField(legacy?"b":"c").get(null);
            Object collision=collisionClass.getMethod("a").invoke(null);
            Method outline=inherited(block.getClass(),legacy?"b":"a",stateClass,viewClass,positionClass,collisionClass);
            Method auxiliary=inherited(block.getClass(),legacy?"a_":"a",stateClass,viewClass,positionClass);
            JsonObject out=new JsonObject();out.addProperty("version",args[0]);out.addProperty("native_class",block.getClass().getName());
            out.addProperty("outline_method_owner",outline.getDeclaringClass().getName());
            out.addProperty("auxiliary_method_owner",auxiliary.getDeclaringClass().getName());
            JsonArray states=new JsonArray(),rays=new JsonArray();
            Object definition=blockClass.getMethod(legacy?"m":"l").invoke(block);
            for(Object state:(List<?>)ExportStorageOutlines.cls("cfk","eoi").getMethod("a").invoke(definition)) {
                JsonObject row=new JsonObject(),nativeState=new JsonObject(),properties=new JsonObject();
                nativeState.addProperty("name","minecraft:crafting_table");
                Map<?,?> entries=(Map<?,?>)ExportStorageOutlines.cls("cfl","eoj").getMethod(legacy?"s":"G").invoke(state);
                for(Map.Entry<?,?> entry:entries.entrySet()) {
                    Class<?> propertyClass=ExportStorageOutlines.cls("cgl","epk");
                    properties.addProperty((String)propertyClass.getMethod("f").invoke(entry.getKey()),
                        (String)propertyClass.getMethod(legacy?"a":"b",Comparable.class).invoke(entry.getKey(),entry.getValue()));
                }
                nativeState.add("properties",properties);row.add("state",nativeState);
                row.addProperty("native_id",(Integer)blockClass.getMethod(legacy?"i":"j",stateClass).invoke(null,state));
                row.add("outline",ExportStorageOutlines.boxes(outline.invoke(block,state,world,origin,collision)));
                row.add("auxiliary",ExportStorageOutlines.boxes(auxiliary.invoke(block,state,world,origin)));
                int index=states.size();states.add(row);
                for(int shift:new int[]{0,29999980}) {
                    for(int axis=0;axis<3;axis++)for(double offset:new double[]{0,1.0/16,.125,.5,.875,15.0/16,1}) {
                        double[] a={offset,offset,offset},b=a.clone();a[axis]=-2;b[axis]=2;
                        rays.add(ExportStorageOutlines.sample(state,air,index,shift,a,b));
                        rays.add(ExportStorageOutlines.sample(state,air,index,shift,b,a));
                    }
                    rays.add(ExportStorageOutlines.sample(state,air,index,shift,new double[]{.5,.5,.5},new double[]{2,2,2}));
                    rays.add(ExportStorageOutlines.sample(state,air,index,shift,new double[]{-2,-2,-2},new double[]{0,0,0}));
                }
            }
            if(states.size()!=1||rays.size()!=88)throw new IllegalStateException("native table state/ray coverage changed");
            out.add("states",states);out.add("rays",rays);
            Files.writeString(Path.of(args[1]),new GsonBuilder().serializeNulls().create().toJson(out)+"\n");
        } finally {if(legacy)Class.forName("v").getMethod("h").invoke(null);}
    }
}
