// Original tooling for unmodified, SHA-1-pinned official server JARs.
// Only bytecode-audited state-only storage outlines are exported. No server runs.
import com.google.gson.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;

public final class ExportStorageOutlines {
    static boolean legacy;
    static Class<?> cls(String oldName, String newName) throws Exception {
        return Class.forName(legacy ? oldName : newName);
    }
    static Object unsafe(Class<?> type) throws Exception {
        Class<?> u = Class.forName("sun.misc.Unsafe");
        Field f = u.getDeclaredField("theUnsafe"); f.setAccessible(true);
        return u.getMethod("allocateInstance", Class.class).invoke(f.get(null), type);
    }
    static void set(Object value, String name, Object data) throws Exception {
        Field f = value.getClass().getDeclaredField(name); f.setAccessible(true); f.set(value, data);
    }
    static JsonArray numbers(double... values) {
        JsonArray out = new JsonArray(); for (double v : values) out.add(v); return out;
    }
    static JsonArray boxes(Object shape) throws Exception {
        JsonArray out = new JsonArray();
        for (Object box : (List<?>) cls("dfg", "fug").getMethod(legacy ? "d" : "e").invoke(shape)) {
            double[] b = new double[6];
            for (int i = 0; i < 6; i++) b[i] = box.getClass().getField("abcdef".substring(i,i+1)).getDouble(box);
            out.add(numbers(b));
        }
        return out;
    }
    static Object context(Object start, Object end) throws Exception {
        Object outline = cls("bpj$a", "dvw$a").getField("b").get(null);
        Object noFluid = cls("bpj$b", "dvw$b").getField("a").get(null);
        Object collision = cls("der", "ftr").getMethod("a").invoke(null);
        Class<?> context = cls("bpj", "dvw");
        if (!legacy) return context.getConstructor(cls("dem","ftm"),cls("dem","ftm"),
            cls("bpj$a","dvw$a"),cls("bpj$b","dvw$b"),cls("der","ftr"))
            .newInstance(start,end,outline,noFluid,collision);
        // Native carrier only: native clip/traversal/shape methods remain unmodified.
        Object value = unsafe(context);
        set(value,"a",start); set(value,"b",end); set(value,"c",outline);
        set(value,"d",noFluid); set(value,"e",collision); return value;
    }
    static JsonObject sample(Object state, Object air, int index, int shift, double[] from, double[] to) throws Exception {
        Class<?> position = cls("fu","is"), view = cls("bpg","dvt"), vector = cls("dem","ftm");
        Object cell = position.getConstructor(int.class,int.class,int.class).newInstance(shift,180,shift);
        Object world = Proxy.newProxyInstance(view.getClassLoader(),new Class<?>[]{view},(proxy,method,args)-> {
            if (method.isDefault()) return InvocationHandler.invokeDefault(proxy,method,args);
            String name = method.getName();
            Object value = args != null && args.length > 0 && cell.equals(args[0]) ? state : air;
            if (name.equals(legacy ? "d_" : "a_")) return value;
            if (name.equals(legacy ? "b" : "b_")) return cls("cfi$a","eog$a")
                .getMethod(legacy ? "m" : "y").invoke(value);
            if (name.equals(legacy ? "c" : "c_")) return null;
            if (name.equals("L_")) return 384;
            if (name.equals("K_")) return -64;
            throw new IllegalStateException("unexpected native world query: "+method);
        });
        double[] start = from.clone(), end = to.clone();
        for (int i=0;i<3;i++) { double offset = i==1 ? 180 : shift; start[i]+=offset; end[i]+=offset; }
        Object a = vector.getConstructor(double.class,double.class,double.class).newInstance(start[0],start[1],start[2]);
        Object b = vector.getConstructor(double.class,double.class,double.class).newInstance(end[0],end[1],end[2]);
        Object hit = view.getMethod("a",cls("bpj","dvw")).invoke(world,context(a,b));
        JsonObject result = new JsonObject();
        result.addProperty("state_index",index);
        JsonArray coordinates = new JsonArray(); coordinates.add(shift); coordinates.add(180); coordinates.add(shift);
        result.add("cell",coordinates);
        result.add("start",numbers(start)); result.add("end",numbers(end));
        if (hit.getClass().getMethod(legacy ? "c" : "d").invoke(hit).toString().equals("BLOCK")) {
            if (!cell.equals(hit.getClass().getMethod(legacy ? "a" : "b").invoke(hit)))
                throw new IllegalStateException("native hit cell differs");
            JsonObject expected = new JsonObject();
            Object p = hit.getClass().getMethod(legacy ? "e" : "g").invoke(hit);
            String fields = legacy ? "bcd" : "ghi";
            double[] xyz = new double[3];
            for (int i=0;i<3;i++) xyz[i]=vector.getField(fields.substring(i,i+1)).getDouble(p);
            expected.add("point",numbers(xyz));
            expected.addProperty("face",hit.getClass().getMethod(legacy ? "b" : "c").invoke(hit).toString().toLowerCase(Locale.ROOT));
            result.add("hit",expected);
        } else result.add("hit",JsonNull.INSTANCE);
        return result;
    }
    public static void main(String[] args) throws Exception {
        legacy = args[0].equals("1.16.1");
        if (!legacy && !args[0].equals("1.21.11")) throw new IllegalArgumentException("version");
        if (!legacy) Class.forName("w").getMethod("a").invoke(null);
        Object version=cls("u","w").getMethod(legacy ? "a" : "b").invoke(null);
        String name=(String)cls("com.mojang.bridge.game.GameVersion","aa")
            .getMethod(legacy ? "getName" : "c").invoke(version);
        if (!name.equals(args[0])) throw new IllegalStateException("native version mismatch");
        cls("uj","amv").getMethod("a").invoke(null);
        try {
            Class<?> registryClass=cls("gl","jq"), blockClass=cls("bvr","dzq"), stateClass=cls("cfj","eoh");
            Object registry=cls("gl","mi").getField(legacy ? "aj" : "e").get(null);
            Method key=registryClass.getMethod("b",Object.class);
            Map<String,Object> blocks=new TreeMap<>();
            for (Object block : (Iterable<?>)registry) blocks.put(key.invoke(registry,block).toString(),block);
            Object air=blockClass.getMethod(legacy ? "n" : "m").invoke(blocks.get("minecraft:air"));
            Object emptyWorld=cls("bpp","dwf").getField("a").get(null);
            Object origin=cls("fu","is").getField(legacy ? "b" : "c").get(null);
            Object emptyCollision=cls("der","ftr").getMethod("a").invoke(null);
            JsonArray states=new JsonArray(), rays=new JsonArray();
            for (String shortName : List.of("chest","trapped_chest","barrel","hopper","dispenser","dropper","ender_chest")) {
                String id="minecraft:"+shortName; Object block=Objects.requireNonNull(blocks.get(id),id);
                Object definition=blockClass.getMethod(legacy ? "m" : "l").invoke(block);
                for (Object state : (List<?>)cls("cfk","eoi").getMethod("a").invoke(definition)) {
                    JsonObject record=new JsonObject(), nativeState=new JsonObject(), properties=new JsonObject();
                    nativeState.addProperty("name",id);
                    Map<?,?> entries=(Map<?,?>)cls("cfl","eoj").getMethod(legacy ? "s" : "G").invoke(state);
                    Map<String,String> ordered=new TreeMap<>();
                    for (Map.Entry<?,?> entry : entries.entrySet()) {
                        Object property=entry.getKey();
                        String p=(String)cls("cgl","epk").getMethod("f").invoke(property);
                        String value=(String)cls("cgl","epk").getMethod(legacy ? "a" : "b",Comparable.class)
                            .invoke(property,entry.getValue()); ordered.put(p,value);
                    }
                    ordered.forEach(properties::addProperty); nativeState.add("properties",properties);
                    record.add("state",nativeState);
                    record.addProperty("native_id",(Integer)blockClass.getMethod(legacy ? "i" : "j",stateClass).invoke(null,state));
                    Object outline=cls("cfi$a","eog$a").getMethod("a",cls("bpg","dvt"),cls("fu","is"),cls("der","ftr"))
                        .invoke(state,emptyWorld,origin,emptyCollision);
                    Object auxiliary=cls("cfi$a","eog$a").getMethod(legacy ? "m" : "i",cls("bpg","dvt"),cls("fu","is"))
                        .invoke(state,emptyWorld,origin);
                    record.add("outline",boxes(outline)); record.add("auxiliary",boxes(auxiliary));
                    int index=states.size(); states.add(record);
                    for (int shift : new int[]{0,29999980}) {
                        for (int axis=0;axis<3;axis++) for (double offset : new double[]{0,1.0/16,0.125,0.5,0.875,15.0/16,1}) {
                            double[] a={offset,offset,offset}, b=a.clone(); a[axis]=-2; b[axis]=2;
                            rays.add(sample(state,air,index,shift,a,b)); rays.add(sample(state,air,index,shift,b,a));
                        }
                        rays.add(sample(state,air,index,shift,new double[]{.5,.5,.5},new double[]{2,2,2}));
                        rays.add(sample(state,air,index,shift,new double[]{-2,-2,-2},new double[]{0,0,0}));
                    }
                }
            }
            if (states.size()!=102) throw new IllegalStateException("native state coverage: "+states.size());
            JsonObject output=new JsonObject(); output.addProperty("version",args[0]);
            output.add("states",states); output.add("rays",rays);
            Files.writeString(Path.of(args[1]),new GsonBuilder().serializeNulls().create().toJson(output)+"\n");
            System.out.println(args[0]+": "+states.size()+" storage states, "+rays.size()+" native clips");
        } finally { if (legacy) Class.forName("v").getMethod("h").invoke(null); }
    }
}
