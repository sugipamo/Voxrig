// Owned observation of unchanged original click/text codecs, getters and equals.
import com.google.gson.*;
import com.mojang.serialization.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.net.URI;
import java.util.*;
import java.io.*;

public final class ExportClickConstructors {
    static JsonObject describeClick(Object value)throws Exception {
        JsonObject out=new JsonObject();String cls=value.getClass().getName();out.addProperty("class",cls);
        Object field=Class.forName(cls).getMethod("b").invoke(value);
        switch(cls) {
            case "yf$g","yf$i" -> out.addProperty("command",(String)field);
            case "yf$c" -> out.addProperty("value",(String)field);
            case "yf$b" -> out.addProperty("page",(int)field);
            case "yf$d" -> {out.addProperty("id",field.toString());Object payload=((Optional<?>)Class.forName(cls).getMethod("c").invoke(value)).orElse(null);out.add("payload",payload==null?JsonNull.INSTANCE:ExportNbtSemantics.describe(payload));}
            default -> throw new IllegalStateException("unreviewed native click getter "+cls);
        }
        return out;
    }
    static void textClicks(Object value,String route,JsonArray out)throws Exception {
        Object contents=Class.forName("yh").getMethod("b").invoke(value),style=Class.forName("yh").getMethod("a").invoke(value);
        Object click=ExportTextCore.field(style,"j");
        if(click!=null) {
            JsonObject row=new JsonObject();row.addProperty("route",route);row.add("fields",describeClick(click));out.add(row);
        }
        switch(contents.getClass().getName()) {
            case "zq" -> {int i=0;for(Object arg:(Object[])ExportTextCore.field(contents,"i")) {if(Class.forName("yh").isInstance(arg))textClicks(arg,route+"/arg"+i,out);i++;}}
            case "zp","zl" -> {String f=contents.getClass().getName().equals("zp")?"c":"e";Object sep=((Optional<?>)ExportTextCore.field(contents,f)).orElse(null);if(sep!=null)textClicks(sep,route+"/separator",out);}
        }
        Object hover=ExportTextCore.field(style,"k");if(hover!=null&&hover.getClass().getName().equals("yo$e"))textClicks(Class.forName("yo$e").getMethod("b").invoke(hover),route+"/hover",out);
        int i=0;for(Object child:(List<?>)Class.forName("yh").getMethod("c").invoke(value))textClicks(child,route+"/sibling"+i++,out);
    }
    @SuppressWarnings("unchecked") public static void main(String[] args)throws Exception {
        Path base=Path.of(args[1]).getParent();
        try(PrintWriter log=new PrintWriter(Files.newBufferedWriter(base.resolve("original-click-bytecode.log")))) {
            int status=java.util.spi.ToolProvider.findFirst("javap").orElseThrow().run(log,log,"-classpath",System.getProperty("java.class.path"),"-p","-c","yf$g","yf$i","yf$c","yf$b","yf$d","bfm","bhi","ym","zl","zs","zv","zw");
            if(status!=0)throw new IllegalStateException("original URI inspection failed");
        }
        ExportItemProperties.init("1.21.11");ExportNbtSemantics.legacy=false;ExportNbtSemantics.tagClass=Class.forName("vz");
        Object type=null;for(Object candidate:(Iterable<?>)ExportItemProperties.components)if(ExportItemProperties.name.invoke(ExportItemProperties.components,candidate).toString().equals("minecraft:custom_name"))type=candidate;
        Object textCodec=Class.forName("kh").getMethod("f").invoke(type);
        JsonObject rules=new JsonObject();JsonArray excluded=new JsonArray();Method allowed=Class.forName("bhi").getMethod("a",int.class);
        for(int c=0;c<65536;c++)if(!(boolean)allowed.invoke(null,c))excluded.add(c);rules.add("chat_excluded_utf16",excluded);
        JsonObject out=new JsonObject();out.add("rules",rules);out.addProperty("java_version",System.getProperty("java.version"));JsonArray cases=new JsonArray();List<Object> values=new ArrayList<>();
        for(JsonElement element:JsonParser.parseString(Files.readString(Path.of(args[0]))).getAsJsonArray()) {
            JsonObject request=element.getAsJsonObject(),row=request.deepCopy();Object value;
            try {
                value=ExportComponentValueRules.decode(textCodec,HexFormat.of().parseHex(request.get("input_hex").getAsString()));
            } catch(InvocationTargetException|IllegalStateException failure) {
                Throwable original=ExportComponentNormalization.cause(failure);if(original instanceof Error error)throw error;row.addProperty("accepted",false);row.addProperty("failure",original.getClass().getName()+": "+original.getMessage());values.add(null);cases.add(row);continue;
            }
            row.addProperty("accepted",true);
            row.add("fields",ExportTextCore.describe(value));JsonArray clicks=new JsonArray();textClicks(value,"root",clicks);row.add("clicks",clicks);
            values.add(value);cases.add(row);
        }
        out.add("cases",cases);long pairs=0;String header=ExportComponentNormalization.jsonUtf16(new GsonBuilder().serializeNulls().setPrettyPrinting().create().toJson(out));
        try(BufferedWriter writer=Files.newBufferedWriter(Path.of(args[1]))) {
            writer.write(header.substring(0,header.length()-1));writer.write(",\"pairs\":[");
            for(int a=0;a<values.size();a++)for(int b=a;b<values.size();b++)if(values.get(a)!=null&&values.get(b)!=null) {if(pairs++!=0)writer.write(",");writer.write("{\"a\":"+a+",\"b\":"+b+",\"equal\":"+values.get(a).equals(values.get(b))+"}");}
            writer.write("]}\n");
        }
        Class.forName("bas").getMethod("close").invoke(ExportItemComponents.resourceManager);System.out.println("original click/text cases="+cases.size()+" pairs="+pairs);
    }
}
