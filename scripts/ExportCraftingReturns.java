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
        if(old&&(row.has("name")||row.has("damage"))) {
            ByteArrayOutputStream bytes=new ByteArrayOutputStream();DataOutputStream out=new DataOutputStream(bytes);
            out.writeByte(10);out.writeUTF("");
            if(row.has("damage")){out.writeByte(3);out.writeUTF("Damage");out.writeInt(row.get("damage").getAsInt());}
            if(row.has("name")) {
                out.writeByte(10);out.writeUTF("display");out.writeByte(8);out.writeUTF("Name");
                out.writeUTF(new Gson().toJson(row.get("name").getAsString()));out.writeByte(0);
            }
            out.writeByte(0);
            JsonObject request=new JsonObject();request.addProperty("nbt_hex",HexFormat.of().formatHex(bytes.toByteArray()));value=ExportItemProperties.decode(ExportItemProperties.input(item,request));
        }
        if(!old&&row.has("capacity")) {
            ByteArrayOutputStream bytes=new ByteArrayOutputStream();DataOutputStream out=new DataOutputStream(bytes);
            ExportItemProperties.varint(out,1);ExportItemProperties.varint(out,0);ExportItemProperties.varint(out,(int)ExportItemProperties.id.invoke(ExportItemProperties.components,types.get("minecraft:max_stack_size")));
            ExportItemProperties.varint(out,row.get("capacity").getAsInt());JsonObject request=new JsonObject();request.addProperty("patch_hex",HexFormat.of().formatHex(bytes.toByteArray()));
            value=ExportItemProperties.decode(ExportItemProperties.input(item,request));
        }
        if(!old)for(String field:List.of("name","damage"))if(row.has(field)) {
            Object type=types.get(field.equals("name")?"minecraft:custom_name":"minecraft:damage");
            Codec<Object> codec=(Codec<Object>)ExportItemProperties.component.getMethod("b").invoke(type);
            Object componentValue=codec.parse(ops,row.get(field)).getOrThrow();
            ExportItemProperties.stack.getMethod("b",ExportItemProperties.component,Object.class).invoke(value,type,componentValue);
        }
        ExportItemProperties.stack.getMethod("e",int.class).invoke(value,row.get("count").getAsInt());return value;
    }
    static Object inventory(JsonObject row,Class<?> type) throws Exception {
        Object value=old?type.getConstructor(Class.forName("bec")).newInstance((Object)null):type.getConstructor(Class.forName("ddm"),Class.forName("cgo")).newInstance(null,Class.forName("cgo").getConstructor().newInstance());
        if(old)ExportInventoryTransfers.field(value,type,"d",row.get("selected").getAsInt());else type.getMethod("d",int.class).invoke(value,row.get("selected").getAsInt());
        for(int index=0;index<36;index++) {
            Object item=stack(row.get("fill"));if(row.has("slots")&&row.getAsJsonObject("slots").has(String.valueOf(index)))item=stack(row.getAsJsonObject("slots").get(String.valueOf(index)));
            type.getMethod("a",int.class,ExportItemProperties.stack).invoke(value,index,item);
        }
        type.getMethod("a",int.class,ExportItemProperties.stack).invoke(value,40,stack(row.has("offhand")?row.get("offhand"):JsonNull.INSTANCE));
        return value;
    }
    static JsonArray contents(Object inventory,Class<?> type) throws Exception {
        JsonArray result=new JsonArray();Method get=type.getMethod("a",int.class);
        for(int index=0;index<36;index++)result.add(ExportItemProperties.encoded(get.invoke(inventory,index)));
        result.add(ExportItemProperties.encoded(get.invoke(inventory,40)));return result;
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
            Object inventory=inventory(row,inventoryClass);
            row.add("initial_encoded",contents(inventory,inventoryClass));
            row.addProperty("inventory_capacity",(Integer)inventoryClass.getMethod(old?"X_":"ap_").invoke(inventory));
            JsonArray inputs=new JsonArray(),remaining=new JsonArray(),moves=new JsonArray(),stranded=new JsonArray();int inputIndex=0;boolean fits=true;
            for(JsonElement input:row.getAsJsonArray("inputs")) {
                Object value=stack(input);inputs.add(ExportItemProperties.encoded(value));int strandedCount=0;
                while((int)count.invoke(value)>0) {
                    int destination=(int)find.invoke(inventory,value);if(destination==-1)destination=(int)free.invoke(inventory);if(destination==-1)break;
                    int room=old?1:(int)stackClass.getMethod("k").invoke(value)-(int)count.invoke(get.invoke(inventory,destination));
                    Object part=stackClass.getMethod("a",int.class).invoke(value,room);
                    int left=(int)count.invoke(part);
                    if(old||(boolean)stackClass.getMethod("n").invoke(part)) {
                        if(!(boolean)inventoryClass.getMethod("c",int.class,stackClass).invoke(inventory,destination,part))throw new IllegalStateException("successful original insert refused");
                        JsonObject move=new JsonObject();move.addProperty("input",inputIndex);move.addProperty("destination",destination);move.addProperty("amount",left-(int)count.invoke(part));moves.add(move);
                    } else while(left>0) {
                        int next=(int)add.invoke(inventory,destination,part);
                        if(next>=left)break;
                        JsonObject move=new JsonObject();move.addProperty("input",inputIndex);move.addProperty("destination",destination);move.addProperty("amount",left-next);moves.add(move);
                        setCount.invoke(part,next);left=next;
                    }
                    int unreturned=(int)count.invoke(part);
                    if(unreturned>0) {
                        strandedCount+=unreturned;JsonObject failure=new JsonObject();failure.addProperty("input",inputIndex);failure.addProperty("destination",destination);failure.addProperty("amount",unreturned);stranded.add(failure);
                    }
                }
                Object retained=stack(input);setCount.invoke(retained,(int)count.invoke(value)+strandedCount);
                if((int)count.invoke(retained)>0)fits=false;
                remaining.add(ExportItemProperties.encoded(retained));inputIndex++;
            }
            JsonArray after=contents(inventory,inventoryClass);
            row.add("inputs_encoded",inputs);row.add("remaining_encoded",remaining);row.add("after_encoded",after);row.add("moves",moves);row.add("stranded",stranded);
            if(!old&&fits) {
                // Execute the entire unchanged original successful handler too.
                // false disables update packets; fitting inputs avoid owner/drop paths.
                Object original=inventory(row,inventoryClass);
                for(JsonElement input:row.getAsJsonArray("inputs"))inventoryClass.getMethod("a",stackClass,boolean.class).invoke(original,stack(input),false);
                JsonArray actual=contents(original,inventoryClass);
                if(!actual.equals(after))throw new IllegalStateException("successful original placeItemBack differs: "+row.get("case"));
                row.add("original_successful_return_encoded",actual);
            }
            cases.add(row);
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
