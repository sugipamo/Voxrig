// Own passive caller of original placement geometry and packet codecs.
// No player/world object, game replacement or independent placement algorithm.
import com.google.gson.*;
import io.netty.buffer.ByteBuf;
import io.netty.buffer.Unpooled;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
public final class ExportRecipePlacement {
    static Object legacyRecipe(JsonObject row) throws Exception {
        Class<?> identity=Class.forName("uh");
        if(row.get("shaped").getAsBoolean()) {
            int width=row.get("recipe_width").getAsInt(),height=row.get("recipe_height").getAsInt();
            Object empty=Class.forName("bmr").getField("a").get(null);
            Object list=Class.forName("gi").getMethod("a",int.class,Object.class).invoke(null,width*height,empty);
            return Class.forName("bmz").getConstructor(identity,String.class,int.class,int.class,Class.forName("gi"),ExportItemProperties.stack)
                .newInstance(identity.getConstructor(String.class).newInstance("voxrig:layout"),"",width,height,list,ExportItemProperties.defaultStack(ExportItemProperties.byName.get("minecraft:stone")));
        }
        Object list=Class.forName("gi").getMethod("a",int.class,Object.class).invoke(null,row.get("entries").getAsInt(),Class.forName("bmr").getField("a").get(null));
        return Class.forName("bna").getConstructor(identity,String.class,ExportItemProperties.stack,Class.forName("gi"))
            .newInstance(identity.getConstructor(String.class).newInstance("voxrig:layout"),"",ExportItemProperties.defaultStack(ExportItemProperties.byName.get("minecraft:stone")),list);
    }
    static JsonArray geometry(JsonObject row,boolean old) throws Exception {
        int width=row.get("grid_width").getAsInt(),height=row.get("grid_height").getAsInt();
        List<Integer> entries=new ArrayList<>();for(int i=0;i<row.get("entries").getAsInt();i++)entries.add(i);
        JsonArray positions=new JsonArray();
        if(old) {
            Class<?> helper=Class.forName("tx");
            Object callback=Proxy.newProxyInstance(helper.getClassLoader(),new Class<?>[]{helper},(proxy,method,args)->{
                if(method.isDefault())return InvocationHandler.invokeDefault(proxy,method,args);
                if(method.getName().equals("a")&&args.length==5) {
                    JsonObject position=new JsonObject();position.addProperty("entry",(Integer)((Iterator<?>)args[0]).next());
                    position.addProperty("native_slot",(Integer)args[1]);position.addProperty("crafts",(Integer)args[2]);
                    position.addProperty("x",(Integer)args[4]);position.addProperty("y",(Integer)args[3]);positions.add(position);return null;
                }
                throw new IllegalStateException("unexpected callback: "+method);
            });
            helper.getMethod("a",int.class,int.class,int.class,Class.forName("bmu"),Iterator.class,int.class)
                .invoke(callback,width,height,0,legacyRecipe(row),entries.iterator(),row.get("crafts").getAsInt());
        } else {
            Class<?> output=Class.forName("ame$a");
            Object callback=Proxy.newProxyInstance(output.getClassLoader(),new Class<?>[]{output},(proxy,method,args)->{
                if(method.getName().equals("addItemToSlot")) {
                    JsonObject position=new JsonObject();position.addProperty("entry",(Integer)args[0]);position.addProperty("grid_index",(Integer)args[1]);
                    position.addProperty("x",(Integer)args[2]);position.addProperty("y",(Integer)args[3]);positions.add(position);return null;
                }
                throw new IllegalStateException("unexpected callback: "+method);
            });
            int rw=row.get("shaped").getAsBoolean()?row.get("recipe_width").getAsInt():width;
            int rh=row.get("shaped").getAsBoolean()?row.get("recipe_height").getAsInt():height;
            Class.forName("ame").getMethod("a",int.class,int.class,int.class,int.class,Iterable.class,output).invoke(null,width,height,rw,rh,entries,callback);
        }
        return positions;
    }
    static JsonObject packet(JsonObject row,boolean old) throws Exception {
        int window=row.get("window").getAsInt();boolean maximum=row.get("maximum").getAsBoolean();
        JsonObject result=new JsonObject();Class<?> type=Class.forName(old?"rw":"ajg");
        if(old) {
            Object value=type.getConstructor().newInstance();
            ExportInventoryTransfers.field(value,type,"a",window);
            ExportInventoryTransfers.field(value,type,"b",Class.forName("uh").getConstructor(String.class).newInstance("minecraft:stick"));
            ExportInventoryTransfers.field(value,type,"c",maximum);
            ByteBuf raw=Unpooled.buffer();try {
                Class<?> bufferType=Class.forName("mg");Object buffer=bufferType.getConstructor(ByteBuf.class).newInstance(raw);
                type.getMethod("b",bufferType).invoke(value,buffer);byte[] bytes=new byte[raw.readableBytes()];raw.getBytes(0,bytes);result.addProperty("encoded_hex",HexFormat.of().formatHex(bytes));
                Object decoded=type.getConstructor().newInstance();type.getMethod("a",bufferType).invoke(decoded,buffer);
                result.addProperty("window",(Integer)type.getMethod("b").invoke(decoded));result.addProperty("recipe",type.getMethod("c").invoke(decoded).toString());
                result.addProperty("maximum",(Boolean)type.getMethod("d").invoke(decoded));if(raw.isReadable())throw new IllegalStateException("trailing native packet");
            }finally{raw.release();}
        } else {
            Object id=Class.forName("dsa").getConstructor(int.class).newInstance(row.get("display_id").getAsInt());
            Object value=type.getConstructor(int.class,Class.forName("dsa"),boolean.class).newInstance(window,id,maximum);
            byte[] bytes=ExportItemComponents.roundtrip(type.getField("a").get(null),value);result.addProperty("encoded_hex",HexFormat.of().formatHex(bytes));
            ByteBuf raw=Unpooled.wrappedBuffer(bytes);try {
                Object decoded=ExportItemProperties.stream.getMethod("decode",Object.class).invoke(type.getField("a").get(null),ExportItemComponents.buffer(raw));
                result.addProperty("window",(Integer)type.getMethod("b").invoke(decoded));result.addProperty("display_id",(Integer)Class.forName("dsa").getMethod("a").invoke(type.getMethod("e").invoke(decoded)));
                result.addProperty("maximum",(Boolean)type.getMethod("f").invoke(decoded));if(raw.isReadable())throw new IllegalStateException("trailing native packet");
            }finally{raw.release();}
        }
        return result;
    }
    public static void main(String[] args) throws Exception {
        String version=args[0];ExportItemProperties.init(version);boolean old=ExportItemProperties.legacy;
        JsonObject out=new JsonObject();out.addProperty("version",version);out.addProperty("java_version",System.getProperty("java.version"));JsonArray cases=new JsonArray();
        for(JsonElement request:new JsonParser().parse(Files.readString(Path.of(args[1]))).getAsJsonArray()) {
            JsonObject row=new JsonParser().parse(request.toString()).getAsJsonObject();
            if(row.get("domain").getAsString().equals("geometry"))row.add("positions",geometry(row,old));else row.add("native_packet",packet(row,old));
            cases.add(row);
        }
        out.add("cases",cases);Files.writeString(Path.of(args[2]),new GsonBuilder().setPrettyPrinting().create().toJson(out)+"\n");
        if(!old)Class.forName("bas").getMethod("close").invoke(ExportItemComponents.resourceManager);
        System.out.println(version+" original placement geometry/codec cases="+cases.size());
    }
}
