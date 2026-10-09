// Own caller of original display constructors and codecs; no game code replacement.
import com.google.gson.*;
import com.mojang.serialization.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import java.io.PrintWriter;
public final class ExportRecipeDisplays {
    @SuppressWarnings("unchecked")
    static Codec<Object> jsonCodec(String domain) throws Exception {
        return (Codec<Object>) Class.forName(domain.equals("slot")?"dse":"dry").getField(domain.equals("slot")?"a":"d").get(null);
    }
    static Object wireCodec(String domain) throws Exception {
        return Class.forName(domain.equals("slot")?"dse":"dry").getField(domain.equals("slot")?"b":"e").get(null);
    }
    static JsonArray registry(String field) throws Exception {
        Object registry=Class.forName("mi").getField(field).get(null);JsonArray out=new JsonArray();
        for(Object value:(Iterable<?>)registry){JsonObject row=new JsonObject();row.addProperty("id",(int)ExportItemProperties.id.invoke(registry,value));row.addProperty("name",ExportItemProperties.name.invoke(registry,value).toString());out.add(row);}
        return out;
    }
    @SuppressWarnings("unchecked")
    public static void main(String[] args) throws Exception {
        ExportItemProperties.init("1.21.11");
        DynamicOps<JsonElement> ops=(DynamicOps<JsonElement>)Class.forName("ams").getMethod("a",DynamicOps.class,Class.forName("jf$a"))
                .invoke(null,JsonOps.INSTANCE,ExportItemComponents.registries);
        JsonObject out=new JsonObject();out.addProperty("version","1.21.11");out.addProperty("java_version",System.getProperty("java.version"));
        out.add("slot_types",registry("az"));out.add("recipe_types",registry("ay"));out.add("categories",registry("aA"));
        JsonArray rows=new JsonArray();
        for(JsonElement request:JsonParser.parseString(Files.readString(Path.of(args[0]))).getAsJsonArray()) {
            JsonObject row=request.getAsJsonObject().deepCopy();String domain=row.get("domain").getAsString();
            DataResult<Object> parsed;
            try {parsed=jsonCodec(domain).parse(ops,row.get("input"));}
            catch(IllegalArgumentException error){row.addProperty("accepted",false);row.addProperty("failure",error.getClass().getName()+": "+error.getMessage());rows.add(row);continue;}
            if(parsed.result().isEmpty()){row.addProperty("accepted",false);row.addProperty("failure",parsed.error().map(e->e.message()).orElse("unknown native parse failure"));rows.add(row);continue;}
            Object value=parsed.result().orElseThrow();row.addProperty("accepted",true);row.addProperty("native_class",value.getClass().getName());
            row.add("native_json",jsonCodec(domain).encodeStart(ops,value).getOrThrow());
            row.addProperty("encoded_hex",HexFormat.of().formatHex(ExportItemComponents.roundtrip(wireCodec(domain),value)));
            if(domain.equals("recipe")) {
                Object category=((Iterable<?>)Class.forName("mi").getField("aA").get(null)).iterator().next();
                Optional<List<Object>> requirements=Optional.empty();
                if(row.has("requirements")){List<Object> list=new ArrayList<>();Codec<Object> ingredient=(Codec<Object>)Class.forName("dqo").getField("d").get(null);for(JsonElement input:row.getAsJsonArray("requirements"))list.add(ingredient.parse(ops,input).getOrThrow());requirements=Optional.of(list);}
                Object identity=Class.forName("dsa").getConstructor(int.class).newInstance(rows.size()+10);
                Object entry=Class.forName("drz").getConstructor(Class.forName("dsa"),Class.forName("dry"),OptionalInt.class,Class.forName("dqv"),Optional.class)
                        .newInstance(identity,value,OptionalInt.of(7),category,requirements);
                row.addProperty("entry_hex",HexFormat.of().formatHex(ExportItemComponents.roundtrip(Class.forName("drz").getField("a").get(null),entry)));
                Object flagged=Class.forName("afs$a").getConstructor(Class.forName("drz"),boolean.class,boolean.class).newInstance(entry,true,false);
                Object packet=Class.forName("afs").getConstructor(List.class,boolean.class).newInstance(List.of(flagged),true);
                row.addProperty("add_packet_hex",HexFormat.of().formatHex(ExportItemComponents.roundtrip(Class.forName("afs").getField("a").get(null),packet)));
            }
            rows.add(row);
        }
        out.add("cases",rows);
        Path base=Path.of(args[1]).getParent();
        try(PrintWriter log=new PrintWriter(Files.newBufferedWriter(base.resolve("recipe-displays-bytecode.log")))) {
            int status=java.util.spi.ToolProvider.findFirst("javap").orElseThrow().run(log,log,"-classpath",System.getProperty("java.class.path"),"-c","-p","dse","dse$a","dse$b","dse$c","dse$d","dse$f","dse$g","dse$h","dse$j","dry","drz","dsa","dsc","dsd","drx","dsh","dsi","afs","afs$a","aft","afu","ajg","dqo");
            if(status!=0)throw new IllegalStateException("original display inspection failed");
        }
        Files.writeString(Path.of(args[1]),new GsonBuilder().setPrettyPrinting().serializeNulls().create().toJson(out)+"\n");
        Class.forName("bas").getMethod("close").invoke(ExportItemComponents.resourceManager);
        System.out.println("original display cases="+rows.size());
    }
}
