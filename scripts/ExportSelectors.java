// Owned observation of unchanged native selector/text constructors and equality.
import com.google.gson.*;
import com.mojang.serialization.*;
import com.mojang.brigadier.StringReader;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import java.io.*;

public final class ExportSelectors {
    @SuppressWarnings("unchecked") public static void main(String[] args)throws Exception {
        Path base=Path.of(args[1]).getParent();
        try(PrintWriter log=new PrintWriter(Files.newBufferedWriter(base.resolve("original-selector-bytecode.log")))) {
            int status=java.util.spi.ToolProvider.findFirst("javap").orElseThrow().run(log,log,"-classpath",System.getProperty("java.class.path"),"-p","-c","gx","gw","gy","gy$b","cq","cq$b","cq$c","cq$d","zo","zs","com.mojang.brigadier.StringReader","aam$16","aam$19","aam$26","wa","vt","vt$a","vt$b","vt$c","vt$d","vt$e","vt$f","vt$g","vt$i","vu","vu$1","vu$2");
            if(status!=0)throw new IllegalStateException("owned original inspection failed");
        }
        ExportItemProperties.init("1.21.11");
        Codec<Object> selector=(Codec<Object>)Class.forName("gx").getField("a").get(null);
        Object type=null;for(Object candidate:(Iterable<?>)ExportItemProperties.components)if(ExportItemProperties.name.invoke(ExportItemProperties.components,candidate).toString().equals("minecraft:custom_name"))type=candidate;
        Object textCodec=Class.forName("kh").getMethod("f").invoke(type);
        JsonObject out=new JsonObject(),rules=new JsonObject();JsonArray options=new JsonArray(),entities=new JsonArray();
        Field optionsField=Class.forName("gy").getDeclaredField("j");optionsField.setAccessible(true);
        for(Object key:new TreeSet<>(((Map<String,?>)optionsField.get(null)).keySet()))options.add(key.toString());
        Object registry=Class.forName("mi").getField("g").get(null);
        for(Object entity:(Iterable<?>)registry)entities.add(ExportItemProperties.name.invoke(registry,entity).toString());
        JsonArray whitespace=new JsonArray(),unquoted=new JsonArray(),numbers=new JsonArray();
        for(int c=0;c<=65535;c++){if(Character.isWhitespace((char)c))whitespace.add(c);if(StringReader.isAllowedInUnquotedString((char)c))unquoted.add(c);if(StringReader.isAllowedNumber((char)c))numbers.add(c);}
        rules.add("whitespace_utf16",whitespace);rules.add("unquoted_utf16",unquoted);rules.add("number_utf16",numbers);
        rules.add("options",options);rules.add("entity_types",entities);out.add("rules",rules);out.addProperty("java_version",System.getProperty("java.version"));
        JsonArray cases=new JsonArray();List<Object> values=new ArrayList<>();
        for(JsonElement input:JsonParser.parseString(Files.readString(Path.of(args[0]))).getAsJsonArray()) {
            JsonObject request=input.getAsJsonObject(),row=request.deepCopy();Object value;
            try {
                if(request.get("kind").getAsString().equals("selector"))value=selector.parse(JsonOps.INSTANCE,request.get("pattern")).getOrThrow();
                else if(request.get("kind").getAsString().equals("profile"))value=ExportComponentValueRules.decode(Class.forName("doy").getField("b").get(null),HexFormat.of().parseHex(request.get("input_hex").getAsString()));
                else value=ExportComponentValueRules.decode(textCodec,HexFormat.of().parseHex(request.get("input_hex").getAsString()));
            } catch(InvocationTargetException|IllegalStateException failure) {
                Throwable original=ExportComponentNormalization.cause(failure);if(original instanceof Error error)throw error;
                row.addProperty("accepted",false);row.addProperty("failure",original.getClass().getName()+": "+original.getMessage());values.add(null);cases.add(row);continue;
            }
            row.addProperty("accepted",true);
            if(request.get("kind").getAsString().equals("selector")) {
                String pattern=(String)Class.forName("gx").getMethod("a").invoke(value);StringReader reader=new StringReader(pattern);
                Object parser=Class.forName("gw").getConstructor(StringReader.class,boolean.class).newInstance(reader,true);
                Object parsed=Class.forName("gw").getMethod("t").invoke(parser);
                JsonObject fields=new JsonObject();fields.addProperty("pattern",pattern);fields.addProperty("cursor",reader.getCursor());
                for(String[] field:new String[][]{{"max_results","d"},{"includes_entities","e"},{"world_limited","f"},{"current_entity","l"},{"player_name","m"},{"uuid","n"},{"uses_selector","p"}})fields.add(field[0],new GsonBuilder().serializeNulls().create().toJsonTree(ExportItemComponentSchema.field(parsed,field[1])));
                row.add("fields",fields);
            } else if(request.get("kind").getAsString().equals("profile"))row.add("fields",ExportProfiles.describe(value));
            else row.add("fields",ExportTextCore.describe(value));
            values.add(value);cases.add(row);
        }
        out.add("cases",cases);long pairs=0;
        String header=ExportComponentNormalization.jsonUtf16(new GsonBuilder().serializeNulls().setPrettyPrinting().create().toJson(out));
        try(BufferedWriter writer=Files.newBufferedWriter(Path.of(args[1]))) {
            writer.write(header.substring(0,header.length()-1));writer.write(",\"pairs\":[");
            for(int a=0;a<values.size();a++)for(int b=a;b<values.size();b++)if(values.get(a)!=null&&values.get(b)!=null) {
                if(pairs++!=0)writer.write(",");
                writer.write("{\"a\":"+a+",\"b\":"+b+",\"equal\":"+values.get(a).equals(values.get(b))+"}");
            }
            writer.write("]}\n");
        }
        Class.forName("bas").getMethod("close").invoke(ExportItemComponents.resourceManager);
        System.out.println("original selector/text cases="+cases.size()+" pairs="+pairs+" options="+options.size()+" entities="+entities.size());
    }
}
