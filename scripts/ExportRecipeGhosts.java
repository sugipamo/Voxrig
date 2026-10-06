// Passive caller of original ghost response codecs; no player/world scaffolding.
import com.google.gson.*;
import com.mojang.serialization.*;
import io.netty.buffer.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
public final class ExportRecipeGhosts {
    static Object field(Object value,Class<?> type,String name) throws Exception {
        Field field=type.getDeclaredField(name);field.setAccessible(true);return field.get(value);
    }
    @SuppressWarnings("unchecked")
    public static void main(String[] args) throws Exception {
        String version=args[0];ExportItemProperties.init(version);boolean old=ExportItemProperties.legacy;
        JsonObject out=new JsonObject();out.addProperty("version",version);out.addProperty("java_version",System.getProperty("java.version"));JsonArray cases=new JsonArray();
        for(JsonElement request:new JsonParser().parse(Files.readString(Path.of(args[1]))).getAsJsonArray()) {
            JsonObject row=new JsonParser().parse(request.toString()).getAsJsonObject();int window=row.get("window").getAsInt();
            if(old && (window < -128 || window > 127))continue;
            Class<?> type=Class.forName(old?"pf":"afg");Object packet;
            ByteBuf raw=Unpooled.buffer();try {
                if(old) {
                    packet=type.getConstructor().newInstance();
                    ExportInventoryTransfers.field(packet,type,"a",window);
                    ExportInventoryTransfers.field(packet,type,"b",Class.forName("uh").getConstructor(String.class).newInstance("minecraft:stick"));
                    Class<?> bufType=Class.forName("mg");Object buf=bufType.getConstructor(ByteBuf.class).newInstance(raw);
                    type.getMethod("b",bufType).invoke(packet,buf);byte[] bytes=new byte[raw.readableBytes()];raw.getBytes(0,bytes);row.addProperty("encoded_hex",HexFormat.of().formatHex(bytes));
                    Object decoded=type.getConstructor().newInstance();type.getMethod("a",bufType).invoke(decoded,buf);
                    row.addProperty("decoded_window",(Integer)field(decoded,type,"a"));row.addProperty("recipe_name",field(decoded,type,"b").toString());
                    Object play=Class.forName("mf").getField("b").get(null),direction=Class.forName("nj").getField("b").get(null);
                    row.addProperty("native_packet_id",(Integer)Class.forName("mf").getMethod("a",Class.forName("nj"),Class.forName("ni")).invoke(play,direction,packet));
                } else {
                    DynamicOps<JsonElement> ops=(DynamicOps<JsonElement>)Class.forName("ams").getMethod("a",DynamicOps.class,Class.forName("jf$a")).invoke(null,JsonOps.INSTANCE,ExportItemComponents.registries);
                    Object display=ExportRecipeDisplays.jsonCodec("recipe").parse(ops,row.get("display")).getOrThrow();
                    packet=type.getConstructor(int.class,Class.forName("dry")).newInstance(window,display);
                    Object codec=type.getField("a").get(null);byte[] bytes=ExportItemComponents.roundtrip(codec,packet);row.addProperty("encoded_hex",HexFormat.of().formatHex(bytes));raw.writeBytes(bytes);
                    Object decoded=ExportItemProperties.stream.getMethod("decode",Object.class).invoke(codec,ExportItemComponents.buffer(raw));
                    row.addProperty("decoded_window",(Integer)type.getMethod("b").invoke(decoded));
                    row.add("native_display",ExportRecipeDisplays.jsonCodec("recipe").encodeStart(ops,type.getMethod("e").invoke(decoded)).getOrThrow());
                    row.addProperty("has_recipe_id",false);
                }
                if(raw.isReadable())throw new IllegalStateException("trailing native ghost packet");
                if(row.get("decoded_window").getAsInt()!=window)throw new IllegalStateException("native ghost window changed");
            }finally{raw.release();}
            cases.add(row);
        }
        out.add("cases",cases);Files.writeString(Path.of(args[2]),new GsonBuilder().setPrettyPrinting().create().toJson(out)+"\n");
        if(!old)Class.forName("bas").getMethod("close").invoke(ExportItemComponents.resourceManager);
        System.out.println(version+" original ghost codec cases="+cases.size());
    }
}
