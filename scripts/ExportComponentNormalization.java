// Own callers of original stream codecs and native value equality; no game bodies.
import com.google.gson.*;
import com.mojang.serialization.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import java.io.*;

public final class ExportComponentNormalization {
    static String jsonUtf16(String json) {
        StringBuilder out=new StringBuilder();
        for(int i=0;i<json.length();i++) {
            char c=json.charAt(i);
            if(Character.isSurrogate(c))out.append(String.format("\\u%04x",(int)c));else out.append(c);
        }
        return out.toString();
    }
    static Throwable cause(Throwable t) {
        while(t instanceof InvocationTargetException)t=t.getCause();
        return t;
    }
    static Object decode(Object codec,byte[] bytes) throws Exception {
        return ExportComponentValueRules.decode(codec,bytes);
    }
    static byte[] encode(Object codec,Object value) throws Exception {
        return ExportItemComponents.roundtrip(codec,value);
    }
    static JsonObject attempt(Object codec,byte[] bytes) throws Exception {
        JsonObject row=new JsonObject();row.addProperty("input_hex",HexFormat.of().formatHex(bytes));
        try {
            Object value=decode(codec,bytes);row.addProperty("accepted",true);
            row.addProperty("value_class",value.getClass().getName());
            row.addProperty("canonical_hex",HexFormat.of().formatHex(encode(codec,value)));
        } catch(InvocationTargetException t) {
            Throwable original=cause(t);
            if(original instanceof Error e)throw e;
            row.addProperty("accepted",false);row.addProperty("failure_stage","stream_decode");row.addProperty("failure",original.getClass().getName()+": "+original.getMessage());
        }
        return row;
    }
    static byte[] item(int componentId,byte[] value) throws Exception {
        ByteArrayOutputStream bytes=new ByteArrayOutputStream();DataOutputStream out=new DataOutputStream(bytes);
        ExportItemProperties.varint(out,1);
        ExportItemProperties.varint(out,(int)ExportItemProperties.id.invoke(ExportItemProperties.items,ExportItemProperties.byName.get("minecraft:stone")));
        ExportItemProperties.varint(out,1);ExportItemProperties.varint(out,0);ExportItemProperties.varint(out,componentId);out.write(value);
        return bytes.toByteArray();
    }
    @SuppressWarnings("unchecked") public static void main(String[] args) throws Exception {
        try(PrintWriter out=new PrintWriter(Files.newBufferedWriter(Path.of(args[1]).getParent().resolve("original-normalization-bytecode.log")))) {
            int result=java.util.spi.ToolProvider.findFirst("javap").orElseThrow().run(out,out,"-classpath",System.getProperty("java.class.path"),"-p","-c","aao$14","aao$17","aam$10","amo","yj","zh","zn","zf","yw");
            if(result!=0)throw new IllegalStateException("own original normalization inspection failed");
        }
        ExportItemProperties.init("1.21.11");
        ExportInventoryTransfers.idOf=ExportItemProperties.id;ExportInventoryTransfers.nameOf=ExportItemProperties.name;
        ExportItemComponentSchema.codec=Class.forName("aao");ExportItemComponentSchema.access=ExportItemComponents.registries;
        JsonObject roots=new JsonObject();Object textType=null;
        for(Object component:(Iterable<?>)ExportItemProperties.components) {
            String name=ExportItemProperties.name.invoke(ExportItemProperties.components,component).toString();
            roots.addProperty(name,ExportItemComponentSchema.node(Class.forName("kh").getMethod("f").invoke(component)));
            if(name.equals("minecraft:custom_name"))textType=component;
        }
        roots.addProperty("native_item",ExportItemComponentSchema.node(ExportItemProperties.stack.getField("h").get(null)));
        JsonObject graph=new JsonObject();graph.add("roots",roots);graph.add("nodes",ExportItemComponentSchema.nodes);
        Object identifier=null,strings=null;
        JsonArray forward=new JsonArray();
        var entries=new ArrayList<>(ExportItemComponentSchema.ids.entrySet());entries.sort(Comparator.comparingInt(Map.Entry::getValue));
        for(var entry:entries) {
            Object codec=entry.getKey();String cls=codec.getClass().getName();
            if(Set.of("aao$14","aao$17","aam$10","aao$11").contains(cls)) {
                JsonObject row=new JsonObject();row.addProperty("node",entry.getValue());row.addProperty("codec_class",cls);
                if(cls.equals("aao$14")||cls.equals("aao$17")) {
                    String decodeField=cls.equals("aao$14")?"a":"b",encodeField=cls.equals("aao$14")?"b":"c";
                    for(String[] field:new String[][]{{"decode",decodeField},{"encode",encodeField}}) {
                        Object function=ExportItemComponentSchema.field(codec,field[1]);
                        row.addProperty(field[0]+"_function_class",function.getClass().getName().split("/",2)[0]);
                    }
                    Object child=ExportItemComponentSchema.field(codec,cls.equals("aao$14")?"c":"a");
                    if(row.get("decode_function_class").getAsString().startsWith("amo$$Lambda")) {
                        if(identifier!=null)throw new IllegalStateException("unreviewed multiple native identifier mappers");
                        identifier=codec;strings=child;row.addProperty("normalization","identifier");
                    }
                }
                forward.add(row);
            }
        }
        if(identifier==null)throw new IllegalStateException("native Identifier forward codec absent");
        JsonArray identifiers=new JsonArray();LinkedHashSet<String> inputs=new LinkedHashSet<>();
        Collections.addAll(inputs,"",":","stone",":stone","minecraft:stone","voxrig:a/b.c-d_e/../","minecraft:","voxrig:","a:b:c","a/b:c","a b:c","minecraft:Stone","MINECRAFT:stone","é:stone","minecraft:é","minecraft:💎","minecraft:\u0000","minecraft:\ud800");
        for(int c=0;c<128;c++) {inputs.add("a"+(char)c+"b:stone");inputs.add("minecraft:a"+(char)c+"b");}
        for(String input:inputs) {
            byte[] wire=encode(strings,input);JsonObject row=attempt(identifier,wire);row.addProperty("input",input);
            if(row.get("accepted").getAsBoolean()) {
                Object value=decode(identifier,wire);row.addProperty("namespace",Class.forName("amo").getMethod("b").invoke(value).toString());row.addProperty("path",Class.forName("amo").getMethod("a").invoke(value).toString());
            }
            identifiers.add(row);
        }
        Object text=Class.forName("kh").getMethod("f").invoke(textType);
        Object nbt=ExportItemComponentSchema.field(text,"a");
        DynamicOps<Object> nbtOps=(DynamicOps<Object>)Class.forName("vn").getField("a").get(null);
        Codec<Object> persistent=(Codec<Object>)Class.forName("kh").getMethod("b").invoke(textType);
        DynamicOps<JsonElement> jsonOps=(DynamicOps<JsonElement>)Class.forName("ams").getMethod("a",DynamicOps.class,Class.forName("jf$a")).invoke(null,JsonOps.INSTANCE,ExportItemComponents.registries);
        int textId=(int)ExportItemProperties.id.invoke(ExportItemProperties.components,textType);
        JsonArray requests=JsonParser.parseString(Files.readString(Path.of(args[0]))).getAsJsonArray();
        JsonArray cases=new JsonArray(),pairs=new JsonArray();List<Object> values=new ArrayList<>(),items=new ArrayList<>();
        for(JsonElement element:requests) {
            JsonObject request=element.getAsJsonObject();JsonElement input=request.get("value");
            Object tag;byte[] wire;
            try {tag=JsonOps.INSTANCE.convertTo(nbtOps,input);}
            catch(RuntimeException original) {
                JsonObject row=new JsonObject();row.addProperty("case",request.get("case").getAsString());row.add("input",input);
                row.addProperty("accepted",false);row.addProperty("failure_stage","input_json_to_nbt");row.addProperty("failure",original.getClass().getName()+": "+original.getMessage());
                cases.add(row);values.add(null);items.add(null);continue;
            }
            try {wire=encode(nbt,tag);}
            catch(InvocationTargetException failure) {
                Throwable original=cause(failure);
                if(!(original instanceof io.netty.handler.codec.EncoderException))throw failure;
                JsonObject row=new JsonObject();row.addProperty("case",request.get("case").getAsString());row.add("input",input);
                row.addProperty("accepted",false);row.addProperty("failure_stage","input_nbt_encode");row.addProperty("failure",original.getClass().getName()+": "+original.getMessage());
                cases.add(row);values.add(null);items.add(null);continue;
            }
            JsonObject row=attempt(text,wire);row.addProperty("case",request.get("case").getAsString());row.add("input",input);
            if(row.get("accepted").getAsBoolean()) {
                Object value=decode(text,wire),independent=decode(text,wire);
                byte[] canonical=HexFormat.of().parseHex(row.get("canonical_hex").getAsString());
                row.addProperty("independent_decode_equal",value.equals(independent));row.addProperty("canonical_decode_equal",value.equals(decode(text,canonical)));
                Object contents=Class.forName("yh").getMethod("b").invoke(value),style=Class.forName("yh").getMethod("a").invoke(value);
                row.addProperty("contents_class",contents.getClass().getName());row.add("canonical_json",persistent.encodeStart(jsonOps,value).getOrThrow());
                JsonObject fields=new JsonObject();
                for(String[] f:new String[][]{{"bold","e"},{"italic","f"},{"underlined","g"},{"strikethrough","h"},{"obfuscated","i"},{"shadow_color","d"},{"insertion","l"}}) {
                    Object v=ExportItemComponentSchema.field(style,f[1]);fields.add(f[0],v==null?JsonNull.INSTANCE:v instanceof Boolean b?new JsonPrimitive(b):v instanceof Number n?new JsonPrimitive(n):new JsonPrimitive(v.toString()));
                }
                Object color=ExportItemComponentSchema.field(style,"c");
                if(color!=null) {JsonObject rgb=new JsonObject();rgb.addProperty("rgb",(int)Class.forName("zh").getMethod("a").invoke(color));rgb.addProperty("serialized",(String)Class.forName("zh").getMethod("b").invoke(color));fields.add("color",rgb);}
                else fields.add("color",JsonNull.INSTANCE);
                row.add("native_style",fields);byte[] item=item(textId,wire);row.addProperty("item_hex",HexFormat.of().formatHex(item));
                values.add(value);items.add(ExportItemProperties.decode(item));
            }else {values.add(null);items.add(null);}
            cases.add(row);
        }
        for(int a=0;a<values.size();a++)for(int b=a;b<values.size();b++)if(values.get(a)!=null&&values.get(b)!=null) {
            JsonObject row=new JsonObject();row.addProperty("a",a);row.addProperty("b",b);row.addProperty("component_equal",values.get(a).equals(values.get(b)));
            row.addProperty("same_item_data",(boolean)ExportItemProperties.stack.getMethod("c",ExportItemProperties.stack,ExportItemProperties.stack).invoke(null,items.get(a),items.get(b)));
            row.addProperty("item_matches",(boolean)ExportItemProperties.stack.getMethod("a",ExportItemProperties.stack,ExportItemProperties.stack).invoke(null,items.get(a),items.get(b)));
            pairs.add(row);
        }
        JsonObject out=new JsonObject();out.add("graph",graph);out.add("forward",forward);out.add("identifiers",identifiers);out.add("text",cases);out.add("text_pairs",pairs);
        Files.writeString(Path.of(args[1]),jsonUtf16(new GsonBuilder().serializeNulls().setPrettyPrinting().create().toJson(out))+"\n");
        System.out.println("original forward nodes="+forward.size()+" identifier cases="+identifiers.size()+" text cases="+cases.size()+" text equality pairs="+pairs.size());
        Class.forName("bas").getMethod("close").invoke(ExportItemComponents.resourceManager);
    }
}
