// Own passive caller of original inventory destination/resource-transfer methods.
// No player/world object or replacement game behavior is created. The original
// Inventory is constructor-initialized with a null unused owner; no owner/world,
// creative fallback, drop or actual recipe operation is called.
import com.google.gson.*;
import com.mojang.serialization.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
public final class ExportCraftingReturns {
    static boolean old;static Map<String,Object> types=new TreeMap<>();static DynamicOps<JsonElement> ops;
    @SuppressWarnings("unchecked")
    static Object stack(JsonElement input) throws Exception {
        if(input.isJsonNull())return ExportItemProperties.stack.getField(old?"b":"l").get(null);
        JsonObject row=input.getAsJsonObject();Object item=Objects.requireNonNull(ExportItemProperties.byName.get(row.get("item").getAsString()));
        Object value=ExportItemProperties.defaultStack(item);
        if(old&&row.has("name")) {
            ByteArrayOutputStream bytes=new ByteArrayOutputStream();DataOutputStream out=new DataOutputStream(bytes);
            out.writeByte(10);out.writeUTF("");out.writeByte(10);out.writeUTF("display");out.writeByte(8);out.writeUTF("Name");
            out.writeUTF(new Gson().toJson(row.get("name").getAsString()));out.writeByte(0);out.writeByte(0);
            JsonObject request=new JsonObject();request.addProperty("nbt_hex",HexFormat.of().formatHex(bytes.toByteArray()));value=ExportItemProperties.decode(ExportItemProperties.input(item,request));
        }
        if(!old&&row.has("name")) {
            Object type=types.get("minecraft:custom_name");Codec<Object> codec=(Codec<Object>)ExportItemProperties.component.getMethod("b").invoke(type);
            Object name=codec.parse(ops,row.get("name")).getOrThrow();ExportItemProperties.stack.getMethod("b",ExportItemProperties.component,Object.class).invoke(value,type,name);
        }
        if(!old&&row.has("capacity")) {
            ByteArrayOutputStream bytes=new ByteArrayOutputStream();DataOutputStream out=new DataOutputStream(bytes);
            ExportItemProperties.varint(out,1);ExportItemProperties.varint(out,0);ExportItemProperties.varint(out,(int)ExportItemProperties.id.invoke(ExportItemProperties.components,types.get("minecraft:max_stack_size")));
            ExportItemProperties.varint(out,row.get("capacity").getAsInt());JsonObject request=new JsonObject();request.addProperty("patch_hex",HexFormat.of().formatHex(bytes.toByteArray()));
            value=ExportItemProperties.decode(ExportItemProperties.input(item,request));
        }
        ExportItemProperties.stack.getMethod("e",int.class).invoke(value,row.get("count").getAsInt());return value;
    }
    @SuppressWarnings("unchecked")
    public static void main(String[] args) throws Exception {
        ExportItemProperties.init(args[0]);old=ExportItemProperties.legacy;
        if(!old){for(Object type:(Iterable<?>)ExportItemProperties.components)types.put(ExportItemProperties.name.invoke(ExportItemProperties.components,type).toString(),type);
            ops=(DynamicOps<JsonElement>)Class.forName("ams").getMethod("a",DynamicOps.class,Class.forName("jf$a")).invoke(null,JsonOps.INSTANCE,ExportItemComponents.registries);}
        Class<?> inventoryClass=Class.forName(old?"beb":"ddl"),stackClass=ExportItemProperties.stack;
        Method find=inventoryClass.getMethod(old?"d":"f",stackClass),free=inventoryClass.getMethod(old?"h":"k"),get=inventoryClass.getMethod("a",int.class),count=stackClass.getMethod(old?"E":"N"),setCount=stackClass.getMethod("e",int.class);
        Method add=inventoryClass.getDeclaredMethod("d",int.class,stackClass);add.setAccessible(true);
        JsonObject out=new JsonObject();out.addProperty("version",args[0]);out.addProperty("java_version",System.getProperty("java.version"));JsonArray cases=new JsonArray();
        for(JsonElement request:new JsonParser().parse(Files.readString(Path.of(args[1]))).getAsJsonArray()) {
            JsonObject row=new JsonParser().parse(request.toString()).getAsJsonObject();
            Object inventory=old?inventoryClass.getConstructor(Class.forName("bec")).newInstance((Object)null):inventoryClass.getConstructor(Class.forName("ddm"),Class.forName("cgo")).newInstance(null,Class.forName("cgo").getConstructor().newInstance());
            if(old)ExportInventoryTransfers.field(inventory,inventoryClass,"d",row.get("selected").getAsInt());else inventoryClass.getMethod("d",int.class).invoke(inventory,row.get("selected").getAsInt());
            JsonArray initial=new JsonArray();for(int index=0;index<36;index++) {
                Object value=stack(row.get("fill"));if(row.has("slots")&&row.getAsJsonObject("slots").has(String.valueOf(index)))value=stack(row.getAsJsonObject("slots").get(String.valueOf(index)));
                inventoryClass.getMethod("a",int.class,stackClass).invoke(inventory,index,value);initial.add(ExportItemProperties.encoded(value));
            }
            Object offhand=stack(row.has("offhand")?row.get("offhand"):JsonNull.INSTANCE);inventoryClass.getMethod("a",int.class,stackClass).invoke(inventory,40,offhand);initial.add(ExportItemProperties.encoded(offhand));row.add("initial_encoded",initial);
            row.addProperty("inventory_capacity",(Integer)inventoryClass.getMethod(old?"X_":"ap_").invoke(inventory));
            JsonArray inputs=new JsonArray(),remaining=new JsonArray(),moves=new JsonArray();int inputIndex=0;
            for(JsonElement input:row.getAsJsonArray("inputs")) {
                Object value=stack(input);inputs.add(ExportItemProperties.encoded(value));
                while((int)count.invoke(value)>0) {
                    int destination=(int)find.invoke(inventory,value);if(destination==-1)destination=(int)free.invoke(inventory);if(destination==-1)break;
                    int before=(int)count.invoke(value),left=(int)add.invoke(inventory,destination,value);if(left>=before)throw new IllegalStateException("original resource transfer made no progress");
                    setCount.invoke(value,left);JsonObject move=new JsonObject();move.addProperty("input",inputIndex);move.addProperty("destination",destination);move.addProperty("amount",before-left);moves.add(move);
                }
                remaining.add(ExportItemProperties.encoded(value));inputIndex++;
            }
            JsonArray after=new JsonArray();for(int index=0;index<36;index++)after.add(ExportItemProperties.encoded(get.invoke(inventory,index)));after.add(ExportItemProperties.encoded(get.invoke(inventory,40)));
            row.add("inputs_encoded",inputs);row.add("remaining_encoded",remaining);row.add("after_encoded",after);row.add("moves",moves);cases.add(row);
        }
        if(!old) {
            JsonObject tags=new JsonObject();
            java.util.stream.Stream<?> lookups=(java.util.stream.Stream<?>)Class.forName("jf$a").getMethod("c").invoke(ExportItemComponents.registries);
            for(Object lookup:lookups.toList()) {
                Object key=Class.forName("jf$b").getMethod("g").invoke(lookup);
                String registry=Class.forName("amt").getMethod("a").invoke(key).toString();
                if(!Set.of("minecraft:block","minecraft:item","minecraft:entity_type").contains(registry))continue;
                JsonObject group=new JsonObject();
                java.util.stream.Stream<?> declared=(java.util.stream.Stream<?>)Class.forName("jf").getMethod("e").invoke(lookup);
                for(Object tag:declared.toList()) {
                    Object tagKey=Class.forName("jh$c").getMethod("h").invoke(tag);
                    String tagName=Class.forName("bef").getMethod("b").invoke(tagKey).toString();
                    JsonArray members=new JsonArray();
                    for(Object holder:(Iterable<?>)tag)members.add(Class.forName("jd").getMethod("g").invoke(holder).toString());
                    group.add(tagName,members);
                }
                tags.add(registry,group);
            }
            out.add("builtin_tags",tags);
        }
        out.add("cases",cases);Files.writeString(Path.of(args[2]),new GsonBuilder().setPrettyPrinting().create().toJson(out)+"\n");
        if(!old)Class.forName("bas").getMethod("close").invoke(ExportItemComponents.resourceManager);
        System.out.println(args[0]+" original resource return cases="+cases.size());
    }
}
