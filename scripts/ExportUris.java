// Owned observation of unchanged original URI/text codecs, getters and equals.
import com.google.gson.*;
import com.mojang.serialization.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.net.URI;
import java.util.*;
import java.io.*;

public final class ExportUris {
    static JsonObject describe(URI value) {
        JsonObject out=new JsonObject();
        out.addProperty("raw",value.toString());out.addProperty("opaque",value.isOpaque());
        for(String[] field:new String[][]{{"scheme",value.getScheme()},{"scheme_specific_part",value.getRawSchemeSpecificPart()},{"authority",value.getRawAuthority()},{"user_info",value.getRawUserInfo()},{"host",value.getHost()},{"path",value.getRawPath()},{"query",value.getRawQuery()},{"fragment",value.getRawFragment()}})
            out.add(field[0],field[1]==null?JsonNull.INSTANCE:new JsonPrimitive(field[1]));
        out.addProperty("port",value.getPort());return out;
    }
    static void textUris(Object value,String route,JsonArray out)throws Exception {
        Object contents=Class.forName("yh").getMethod("b").invoke(value),style=Class.forName("yh").getMethod("a").invoke(value);
        Object click=ExportTextCore.field(style,"j");
        if(click!=null&&click.getClass().getName().equals("yf$f")) {
            JsonObject row=new JsonObject();row.addProperty("route",route);row.add("fields",describe((URI)Class.forName("yf$f").getMethod("b").invoke(click)));out.add(row);
        }
        switch(contents.getClass().getName()) {
            case "zq" -> {int i=0;for(Object arg:(Object[])ExportTextCore.field(contents,"i")) {if(Class.forName("yh").isInstance(arg))textUris(arg,route+"/arg"+i,out);i++;}}
            case "zp","zl" -> {String f=contents.getClass().getName().equals("zp")?"c":"e";Object sep=((Optional<?>)ExportTextCore.field(contents,f)).orElse(null);if(sep!=null)textUris(sep,route+"/separator",out);}
        }
        Object hover=ExportTextCore.field(style,"k");if(hover!=null&&hover.getClass().getName().equals("yo$e"))textUris(Class.forName("yo$e").getMethod("b").invoke(hover),route+"/hover",out);
        int i=0;for(Object child:(List<?>)Class.forName("yh").getMethod("c").invoke(value))textUris(child,route+"/sibling"+i++,out);
    }
    @SuppressWarnings("unchecked") public static void main(String[] args)throws Exception {
        Path base=Path.of(args[1]).getParent();
        try(PrintWriter log=new PrintWriter(Files.newBufferedWriter(base.resolve("original-uri-bytecode.log")))) {
            int status=java.util.spi.ToolProvider.findFirst("javap").orElseThrow().run(log,log,"-classpath",System.getProperty("java.class.path"),"-p","-c","yf$f","bfm","bhs","java.net.URI","java.net.URI$Parser");
            if(status!=0)throw new IllegalStateException("original URI inspection failed");
        }
        ExportItemProperties.init("1.21.11");Codec<Object> codec=(Codec<Object>)Class.forName("bfm").getField("P").get(null);
        Object type=null;for(Object candidate:(Iterable<?>)ExportItemProperties.components)if(ExportItemProperties.name.invoke(ExportItemProperties.components,candidate).toString().equals("minecraft:custom_name"))type=candidate;
        Object textCodec=Class.forName("kh").getMethod("f").invoke(type);
        JsonObject rules=new JsonObject(),masks=new JsonObject();
        for(String name:new String[]{"URIC","PATH","USERINFO","REG_NAME","SERVER","SERVER_PERCENT","SCHEME","SCOPE_ID"}) {
            Field lf=URI.class.getDeclaredField("L_"+name),hf=URI.class.getDeclaredField("H_"+name);lf.setAccessible(true);hf.setAccessible(true);long low=lf.getLong(null),high=hf.getLong(null);
            JsonObject mask=new JsonObject();JsonArray chars=new JsonArray();for(int c=1;c<128;c++)if((((c<64?low:high) >>> (c%64))&1)!=0)chars.add(c);
            mask.add("ascii_units",chars);mask.addProperty("escaped",(low&1)!=0);masks.add(name,mask);
        }
        rules.add("masks",masks);JsonArray excluded=new JsonArray();for(int c=128;c<65536;c++)if(Character.isSpaceChar((char)c)||Character.isISOControl((char)c))excluded.add(c);rules.add("excluded_non_ascii_units",excluded);
        Field schemes=Class.forName("bhs").getDeclaredField("o");schemes.setAccessible(true);JsonArray allowed=new JsonArray();for(Object scheme:new TreeSet<>((Set<String>)schemes.get(null)))allowed.add(scheme.toString());rules.add("allowed_schemes",allowed);
        JsonObject out=new JsonObject();out.add("rules",rules);out.addProperty("java_version",System.getProperty("java.version"));JsonArray cases=new JsonArray();List<Object> values=new ArrayList<>();
        for(JsonElement element:JsonParser.parseString(Files.readString(Path.of(args[0]))).getAsJsonArray()) {
            JsonObject request=element.getAsJsonObject(),row=request.deepCopy();Object value;
            try {
                if(request.get("kind").getAsString().equals("uri"))value=codec.parse(JsonOps.INSTANCE,request.get("value")).getOrThrow();
                else value=ExportComponentValueRules.decode(textCodec,HexFormat.of().parseHex(request.get("input_hex").getAsString()));
            } catch(InvocationTargetException|IllegalStateException failure) {
                Throwable original=ExportComponentNormalization.cause(failure);if(original instanceof Error error)throw error;row.addProperty("accepted",false);row.addProperty("failure",original.getClass().getName()+": "+original.getMessage());values.add(null);cases.add(row);continue;
            }
            row.addProperty("accepted",true);
            if(value instanceof URI uri)row.add("fields",describe(uri));
            else {row.add("fields",ExportTextCore.describe(value));JsonArray uris=new JsonArray();textUris(value,"root",uris);row.add("uri_clicks",uris);}
            values.add(value);cases.add(row);
        }
        out.add("cases",cases);long pairs=0;String header=ExportComponentNormalization.jsonUtf16(new GsonBuilder().serializeNulls().setPrettyPrinting().create().toJson(out));
        try(BufferedWriter writer=Files.newBufferedWriter(Path.of(args[1]))) {
            writer.write(header.substring(0,header.length()-1));writer.write(",\"pairs\":[");
            for(int a=0;a<values.size();a++)for(int b=a;b<values.size();b++)if(values.get(a)!=null&&values.get(b)!=null) {if(pairs++!=0)writer.write(",");writer.write("{\"a\":"+a+",\"b\":"+b+",\"equal\":"+values.get(a).equals(values.get(b))+"}");}
            writer.write("]}\n");
        }
        Class.forName("bas").getMethod("close").invoke(ExportItemComponents.resourceManager);System.out.println("original URI/text cases="+cases.size()+" pairs="+pairs);
    }
}
