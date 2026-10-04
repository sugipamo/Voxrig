// Owned observer of unchanged original constructor codecs; no game method replacement.
import com.google.gson.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import java.io.*;

public final class ExportTextConstructors {
    static Object staticField(String cls, String name) throws Exception {
        Field field=Class.forName(cls).getDeclaredField(name);field.setAccessible(true);return field.get(null);
    }
    static JsonArray names(Object mapper) throws Exception {
        JsonArray out=new JsonArray();
        for(Object name:((Map<?,?>)ExportItemComponentSchema.field(mapper,"a")).keySet())out.add(name.toString());
        return out;
    }
    public static void main(String[] args) throws Exception {
        Path base=Path.of(args[1]).getParent();
        try(PrintWriter log=new PrintWriter(Files.newBufferedWriter(base.resolve("original-constructor-bytecode.log")))) {
            int status=java.util.spi.ToolProvider.findFirst("javap").orElseThrow().run(log,log,"-classpath",System.getProperty("java.class.path"),"-p","-c","yj","yj$a","yj$b","bfm$b","zu","aaa","zq","aab","yf","com.mojang.serialization.codecs.OptionalFieldCodec");
            if(status!=0)throw new IllegalStateException("owned original inspection failed");
        }
        ExportItemProperties.init("1.21.11");
        JsonObject out=new JsonObject(),rules=new JsonObject();
        Class<?> mapperClass=Class.forName("bfm$b");Object mapper=mapperClass.getConstructor().newInstance();
        Method bootstrap=Class.forName("yj").getDeclaredMethod("a",mapperClass);bootstrap.setAccessible(true);bootstrap.invoke(null,mapper);
        rules.add("contents",names(mapper));
        rules.add("sources",names(staticField("zu","b")));
        rules.add("objects",names(staticField("aaa","b")));out.add("rules",rules);
        Object type=null;for(Object candidate:(Iterable<?>)ExportItemProperties.components)if(ExportItemProperties.name.invoke(ExportItemProperties.components,candidate).toString().equals("minecraft:custom_name"))type=candidate;
        Object codec=Class.forName("kh").getMethod("f").invoke(type);
        JsonArray cases=new JsonArray();List<Object> values=new ArrayList<>();
        for(JsonElement input:JsonParser.parseString(Files.readString(Path.of(args[0]))).getAsJsonArray()) {
            JsonObject request=input.getAsJsonObject(),row=request.deepCopy();
            try {
                Object value=ExportComponentValueRules.decode(codec,HexFormat.of().parseHex(request.get("input_hex").getAsString()));
                row.addProperty("accepted",true);row.add("fields",ExportTextCore.describe(value));values.add(value);
            } catch(InvocationTargetException failure) {
                Throwable original=ExportComponentNormalization.cause(failure);if(original instanceof Error error)throw error;
                row.addProperty("accepted",false);row.addProperty("failure",original.getClass().getName()+": "+original.getMessage());values.add(null);
            }
            cases.add(row);
        }
        JsonArray pairs=new JsonArray();for(int a=0;a<values.size();a++)for(int b=a;b<values.size();b++)if(values.get(a)!=null&&values.get(b)!=null) {
            JsonObject pair=new JsonObject();pair.addProperty("a",a);pair.addProperty("b",b);pair.addProperty("equal",values.get(a).equals(values.get(b)));pairs.add(pair);
        }
        out.add("cases",cases);out.add("pairs",pairs);
        Files.writeString(Path.of(args[1]),ExportComponentNormalization.jsonUtf16(new GsonBuilder().serializeNulls().setPrettyPrinting().create().toJson(out))+"\n");
        Class.forName("bas").getMethod("close").invoke(ExportItemComponents.resourceManager);
        System.out.println("original constructor cases="+cases.size()+" pairs="+pairs.size()+" rules="+rules);
    }
}
