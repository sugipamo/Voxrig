// Own read-only wrapper of original stack getters and stock accounting.
// Original game classes/methods are neither replaced nor stubbed.
import com.google.gson.*;
import com.mojang.serialization.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;

public final class ExportRecipeBookStock {
    @SuppressWarnings("unchecked")
    public static void main(String[] args) throws Exception {
        String version=args[0];ExportItemProperties.init(version);boolean old=ExportItemProperties.legacy;
        Class<?> stack=ExportItemProperties.stack;
        Map<String,Object> types=new TreeMap<>();DynamicOps<JsonElement> ops=null;
        if(!old) {
            for(Object type:(Iterable<?>)ExportItemProperties.components)types.put(ExportItemProperties.name.invoke(ExportItemProperties.components,type).toString(),type);
            ops=(DynamicOps<JsonElement>)Class.forName("ams").getMethod("a",DynamicOps.class,Class.forName("jf$a")).invoke(null,JsonOps.INSTANCE,ExportItemComponents.registries);
        }
        JsonObject out=new JsonObject();out.addProperty("version",version);out.addProperty("java_version",System.getProperty("java.version"));JsonArray cases=new JsonArray();
        for(JsonElement request:new JsonParser().parse(Files.readString(Path.of(args[1]))).getAsJsonObject().getAsJsonArray(version)) {
            JsonObject row=new JsonParser().parse(request.toString()).getAsJsonObject();Object item=ExportItemProperties.byName.get(row.get("item").getAsString());if(item==null)throw new IllegalArgumentException("unknown item");
            Object value;
            if(old)value=ExportItemProperties.decode(ExportItemProperties.input(item,row));
            else {
                value=ExportItemProperties.defaultStack(item);
                if(row.has("components"))for(var entry:row.getAsJsonObject("components").entrySet()) {
                    Object type=Objects.requireNonNull(types.get(entry.getKey()));
                    Codec<Object> codec=(Codec<Object>)ExportItemProperties.component.getMethod("b").invoke(type);
                    Object field=codec.parse(ops,entry.getValue()).getOrThrow();
                    stack.getMethod("b",ExportItemProperties.component,Object.class).invoke(value,type,field);
                }
                if(row.has("patch_hex"))value=ExportItemProperties.decode(ExportItemProperties.input(item,row));
                if(row.has("removed"))for(JsonElement name:row.getAsJsonArray("removed"))stack.getMethod("e",ExportItemProperties.component).invoke(value,Objects.requireNonNull(types.get(name.getAsString())));
            }
            stack.getMethod("e",int.class).invoke(value,row.get("count").getAsInt());
            row.addProperty("encoded_item_hex",ExportItemProperties.encoded(value));row.add("properties",ExportItemProperties.properties(value));
            row.addProperty("damaged",(boolean)stack.getMethod(old?"f":"n").invoke(value));
            row.addProperty("enchanted",(boolean)stack.getMethod(old?"x":"F").invoke(value));
            boolean named=old?(boolean)stack.getMethod("t").invoke(value):(boolean)stack.getMethod("c",ExportItemProperties.component).invoke(value,types.get("minecraft:custom_name"));row.addProperty("custom_named",named);
            Object stocks=Class.forName(old?"bee":"ddu").getConstructor().newInstance();stocks.getClass().getMethod("a",stack).invoke(stocks,value);
            Object map,key;
            if(old) {map=stocks.getClass().getField("a").get(stocks);key=(int)ExportItemProperties.id.invoke(ExportItemProperties.items,item);}
            else {
                Field raw=stocks.getClass().getDeclaredField("a");raw.setAccessible(true);Object counts=raw.get(stocks);
                Field amounts=counts.getClass().getDeclaredField("a");amounts.setAccessible(true);map=amounts.get(counts);key=stack.getMethod("i").invoke(value);
            }
            int contribution=(int)map.getClass().getMethod(old?"get":"getInt",old?int.class:Object.class).invoke(map,key);
            row.addProperty("accounted_count",contribution);cases.add(row);
        }
        out.add("cases",cases);Files.writeString(Path.of(args[2]),new GsonBuilder().setPrettyPrinting().create().toJson(out)+"\n");
        if(!old)Class.forName("bas").getMethod("close").invoke(ExportItemComponents.resourceManager);
        System.out.println(version+" native recipe book stock cases="+cases.size());
    }
}
