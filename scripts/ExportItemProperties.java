// Own wrapper of unchanged original item constructors, codecs and property getters.
// No skeletal player/world is needed, and no game method is replaced.
import com.google.gson.*;
import io.netty.buffer.ByteBuf;
import io.netty.buffer.Unpooled;
import java.lang.reflect.*;
import java.nio.file.*;
import java.io.*;
import java.util.*;

public final class ExportItemProperties {
    static boolean legacy;
    static Class<?> stack, registry, component, stream;
    static Object items, components;
    static Method id, name;
    static Map<String,Object> byName = new TreeMap<>();
    static void init(String version) throws Exception {
        legacy = version.equals("1.16.1");
        if (!legacy && !version.equals("1.21.11")) throw new IllegalArgumentException("version");
        if (!legacy) Class.forName("w").getMethod("a").invoke(null);
        Object actual = Class.forName(legacy ? "u" : "w").getMethod(legacy ? "a" : "b").invoke(null);
        if (!Class.forName(legacy ? "com.mojang.bridge.game.GameVersion" : "aa")
                .getMethod(legacy ? "getName" : "c").invoke(actual).equals(version))
            throw new IllegalStateException("version mismatch");
        Class.forName(legacy ? "uj" : "amv").getMethod("a").invoke(null);
        stack = Class.forName(legacy ? "bki" : "dlt"); registry = Class.forName(legacy ? "gl" : "jq");
        id = registry.getMethod("a", Object.class); name = registry.getMethod("b", Object.class);
        items = Class.forName(legacy ? "gl" : "mi").getField(legacy ? "am" : "h").get(null);
        for (Object item : (Iterable<?>) items) byName.put(name.invoke(items,item).toString(),item);
        if (!legacy) {
            // Reuse only the unchanged original resource/registry/tag loader wrapper.
            ExportInventoryTransfers.registryClass = registry;
            ExportItemComponents.loadVanillaRegistries();
            component = Class.forName("kh"); stream = Class.forName("aao");
            components = Class.forName("mi").getField("am").get(null);
            ExportItemComponents.codecClass = stream; ExportItemComponents.bufferClass = Class.forName("xq");
        }
    }
    static Object defaultStack(Object item) throws Exception {
        return stack.getConstructor(Class.forName(legacy ? "bqa" : "dwn"), int.class).newInstance(item,1);
    }
    static JsonObject properties(Object value) throws Exception {
        JsonObject result = new JsonObject();
        String[][] ints = {{"max_stack_size",legacy?"c":"k"},{"max_damage",legacy?"h":"p"},{"damage",legacy?"g":"o"}};
        String[][] bools = {{"stackable",legacy?"d":"l"},{"damageable",legacy?"e":"m"},{"damaged",legacy?"f":"n"}};
        for (String[] field : ints) result.addProperty(field[0],(int)stack.getMethod(field[1]).invoke(value));
        for (String[] field : bools) result.addProperty(field[0],(boolean)stack.getMethod(field[1]).invoke(value));
        return result;
    }
    static void varint(DataOutputStream out,int value) throws Exception {
        do { int b=value&127;value>>>=7;out.writeByte(b|(value==0?0:128)); } while(value!=0);
    }
    static byte[] input(Object item,JsonObject request) throws Exception {
        ByteArrayOutputStream bytes=new ByteArrayOutputStream();DataOutputStream out=new DataOutputStream(bytes);
        if (legacy) {out.writeBoolean(true);varint(out,(int)id.invoke(items,item));out.writeByte(1);out.write(HexFormat.of().parseHex(request.get("nbt_hex").getAsString()));}
        else {varint(out,1);varint(out,(int)id.invoke(items,item));out.write(HexFormat.of().parseHex(request.get("patch_hex").getAsString()));}
        return bytes.toByteArray();
    }
    static Object decode(byte[] input) throws Exception {
        ByteBuf bytes=Unpooled.wrappedBuffer(input);
        try {
            Object value;
            if (legacy) {Object buffer=Class.forName("mg").getConstructor(ByteBuf.class).newInstance(bytes);value=Class.forName("mg").getMethod("m").invoke(buffer);}
            else value=stream.getMethod("decode",Object.class).invoke(stack.getField("h").get(null),ExportItemComponents.buffer(bytes));
            if(bytes.isReadable())throw new IllegalStateException("native item left trailing bytes");
            return value;
        } finally {bytes.release();}
    }
    static String encoded(Object value) throws Exception {
        if (!legacy) return HexFormat.of().formatHex(ExportItemComponents.roundtrip(stack.getField("h").get(null),value));
        ByteBuf bytes=Unpooled.buffer();
        try {Object buffer=Class.forName("mg").getConstructor(ByteBuf.class).newInstance(bytes);Class.forName("mg").getMethod("a",stack).invoke(buffer,value);byte[] out=new byte[bytes.readableBytes()];bytes.getBytes(0,out);return HexFormat.of().formatHex(out);}
        finally {bytes.release();}
    }
    public static void main(String[] args) throws Exception {
        init(args[0]);JsonObject output=new JsonObject();output.addProperty("version",args[0]);
        JsonArray defaults=new JsonArray(),values=new JsonArray(),cases=new JsonArray(),failures=new JsonArray();
        Map<String,Integer> fingerprints=new HashMap<>();
        for (var item : byName.entrySet()) {
            Object value=defaultStack(item.getValue());JsonObject row=new JsonObject();row.addProperty("name",item.getKey());row.addProperty("native_id",(int)id.invoke(items,item.getValue()));
            row.addProperty("represents_empty",(boolean)stack.getMethod(legacy?"a":"f").invoke(value));
            row.add("properties",properties(value));row.addProperty("encoded_item_hex",encoded(value));
            if(legacy) row.addProperty("normalizes_damage_on_read",(boolean)Class.forName("bke").getMethod("k").invoke(item.getValue()));
            else {
                JsonArray indices=new JsonArray();JsonObject simple=new JsonObject();
                Object prototype=stack.getMethod("c").invoke(value);
                for(Object typed : (Iterable<?>)prototype) {
                    Object type=Class.forName("kk").getMethod("a").invoke(typed),v=Class.forName("kk").getMethod("b").invoke(typed);
                    String typeName=name.invoke(components,type).toString();int typeId=(int)id.invoke(components,type);
                    String hex=HexFormat.of().formatHex(ExportItemComponents.roundtrip(component.getMethod("f").invoke(type),v));
                    String key=typeId+":"+hex;Integer index=fingerprints.get(key);
                    if(index==null) {index=values.size();fingerprints.put(key,index);JsonObject fact=new JsonObject();fact.addProperty("name",typeName);fact.addProperty("native_id",typeId);fact.addProperty("value_hex",hex);values.add(fact);}
                    indices.add(index);
                    if(List.of("minecraft:max_stack_size","minecraft:max_damage","minecraft:damage").contains(typeName)) simple.addProperty(typeName,(Integer)v);
                    if(typeName.equals("minecraft:unbreakable"))simple.addProperty(typeName,true);
                }
                List<Integer> sorted=new ArrayList<>();for(JsonElement i:indices)sorted.add(i.getAsInt());Collections.sort(sorted);indices=new JsonArray();for(int i:sorted)indices.add(i);
                row.add("prototype_values",indices);row.add("property_components",simple);
            }
            defaults.add(row);
        }
        // Native reference-map iteration is not a stable fixture ordering.
        List<Integer> order=new ArrayList<>();for(int i=0;i<values.size();i++)order.add(i);
        final JsonArray unsortedValues=values;
        order.sort(Comparator.comparingInt((Integer i)->unsortedValues.get(i).getAsJsonObject().get("native_id").getAsInt())
                .thenComparing(i->unsortedValues.get(i).getAsJsonObject().get("value_hex").getAsString()));
        int[] remap=new int[values.size()];values=new JsonArray();
        for(int old:order) {remap[old]=values.size();values.add(unsortedValues.get(old));}
        if(!legacy)for(JsonElement element:defaults) {
            JsonObject row=element.getAsJsonObject();List<Integer> indices=new ArrayList<>();
            for(JsonElement i:row.getAsJsonArray("prototype_values"))indices.add(remap[i.getAsInt()]);
            Collections.sort(indices);JsonArray sorted=new JsonArray();for(int i:indices)sorted.add(i);
            row.add("prototype_values",sorted);
        }
        JsonArray requests=new JsonParser().parse(Files.readString(Path.of(args[1]))).getAsJsonObject().getAsJsonArray(args[0]);
        for(JsonElement element:requests) {
            JsonObject request=element.getAsJsonObject();JsonObject row=new JsonObject();row.addProperty("case",request.get("case").getAsString());row.addProperty("item",request.get("item").getAsString());
            Object item=byName.get(row.get("item").getAsString());if(item==null)throw new IllegalStateException("unknown requested item");
            byte[] input=input(item,request);row.addProperty("input_item_hex",HexFormat.of().formatHex(input));row.addProperty("native_id",(int)id.invoke(items,item));
            try {Object decoded=decode(input);row.add("properties",properties(decoded));row.addProperty("canonical_item_hex",encoded(decoded));cases.add(row);}
            catch(InvocationTargetException e) {row.addProperty("failure",e.getCause().getClass().getName()+": "+e.getCause().getMessage());failures.add(row);}
        }
        output.add("defaults",defaults);output.add("prototype_values",values);output.add("cases",cases);output.add("failures",failures);
        Files.writeString(Path.of(args[2]),new GsonBuilder().setPrettyPrinting().create().toJson(output)+"\n");
        System.out.println(args[0]+" defaults="+defaults.size()+" prototype values="+values.size()+" cases="+cases.size()+" failures="+failures.size());
        if(ExportItemComponents.resourceManager!=null)Class.forName("bas").getMethod("close").invoke(ExportItemComponents.resourceManager);
    }
}
