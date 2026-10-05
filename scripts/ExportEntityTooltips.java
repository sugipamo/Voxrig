// Owned observation of unchanged original entity tooltip/text codecs, getters and equals.
import com.google.gson.*;
import com.mojang.serialization.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.net.URI;
import java.util.*;
import java.io.*;

public final class ExportEntityTooltips {
    static void collect(Object value,String route,JsonArray out)throws Exception {
        Object contents=Class.forName("yh").getMethod("b").invoke(value),style=Class.forName("yh").getMethod("a").invoke(value);
        Object hover=ExportTextCore.field(style,"k");
        if(hover!=null&&hover.getClass().getName().equals("yo$c")) {
            Object info=Class.forName("yo$c").getMethod("b").invoke(hover),type=ExportTextCore.field(info,"b"),uuid=ExportTextCore.field(info,"c"),name=((Optional<?>)ExportTextCore.field(info,"d")).orElse(null);
            Object registry=Class.forName("mi").getField("g").get(null);JsonObject fields=new JsonObject();fields.addProperty("id",ExportItemProperties.name.invoke(registry,type).toString());fields.add("uuid",new Gson().toJsonTree(Class.forName("jx").getMethod("a",java.util.UUID.class).invoke(null,uuid)));fields.add("name",name==null?JsonNull.INSTANCE:ExportTextCore.describe(name));
            JsonObject row=new JsonObject();row.addProperty("route",route);row.add("fields",fields);out.add(row);if(name!=null)collect(name,route+"/name",out);
        }
        if(hover!=null&&hover.getClass().getName().equals("yo$e"))collect(Class.forName("yo$e").getMethod("b").invoke(hover),route+"/hover",out);
        switch(contents.getClass().getName()) {
            case "zq" -> {int i=0;for(Object arg:(Object[])ExportTextCore.field(contents,"i")){if(Class.forName("yh").isInstance(arg))collect(arg,route+"/arg"+i,out);i++;}}
            case "zp","zl" -> {Object sep=((Optional<?>)ExportTextCore.field(contents,contents.getClass().getName().equals("zp")?"c":"e")).orElse(null);if(sep!=null)collect(sep,route+"/separator",out);}
        }
        int i=0;for(Object child:(List<?>)Class.forName("yh").getMethod("c").invoke(value))collect(child,route+"/sibling"+i++,out);
    }
    @SuppressWarnings("unchecked") public static void main(String[] args)throws Exception {
        Path base=Path.of(args[1]).getParent();
        try(PrintWriter log=new PrintWriter(Files.newBufferedWriter(base.resolve("original-entity-tooltip-bytecode.log")))) {
            int status=java.util.spi.ToolProvider.findFirst("javap").orElseThrow().run(log,log,"-classpath",System.getProperty("java.class.path"),"-p","-c","yo$b","yo$c","jx","java.util.UUID","mi","iy");
            if(status!=0)throw new IllegalStateException("original URI inspection failed");
        }
        ExportItemProperties.init("1.21.11");ExportNbtSemantics.legacy=false;ExportNbtSemantics.tagClass=Class.forName("vz");
        Object type=null;for(Object candidate:(Iterable<?>)ExportItemProperties.components)if(ExportItemProperties.name.invoke(ExportItemProperties.components,candidate).toString().equals("minecraft:custom_name"))type=candidate;
        Object textCodec=Class.forName("kh").getMethod("f").invoke(type);
        JsonObject rules=new JsonObject();JsonArray types=new JsonArray();Object registry=Class.forName("mi").getField("g").get(null);for(Object typeEntry:(Iterable<?>)registry)types.add(ExportItemProperties.name.invoke(registry,typeEntry).toString());rules.add("entity_types",types);
        JsonObject out=new JsonObject();out.add("rules",rules);out.addProperty("java_version",System.getProperty("java.version"));JsonArray cases=new JsonArray();List<Object> values=new ArrayList<>();
        for(JsonElement element:JsonParser.parseString(Files.readString(Path.of(args[0]))).getAsJsonArray()) {
            JsonObject request=element.getAsJsonObject(),row=request.deepCopy();Object value;
            try {
                value=ExportComponentValueRules.decode(textCodec,HexFormat.of().parseHex(request.get("input_hex").getAsString()));
            } catch(InvocationTargetException|IllegalStateException failure) {
                Throwable original=ExportComponentNormalization.cause(failure);if(original instanceof Error error)throw error;row.addProperty("accepted",false);row.addProperty("failure",original.getClass().getName()+": "+original.getMessage());values.add(null);cases.add(row);continue;
            }
            row.addProperty("accepted",true);
            row.add("fields",ExportTextCore.describe(value));JsonArray entities=new JsonArray();collect(value,"root",entities);row.add("entities",entities);
            values.add(value);cases.add(row);
        }
        out.add("cases",cases);long pairs=0;String header=ExportComponentNormalization.jsonUtf16(new GsonBuilder().serializeNulls().setPrettyPrinting().create().toJson(out));
        try(BufferedWriter writer=Files.newBufferedWriter(Path.of(args[1]))) {
            writer.write(header.substring(0,header.length()-1));writer.write(",\"pairs\":[");
            for(int a=0;a<values.size();a++)for(int b=a;b<values.size();b++)if(values.get(a)!=null&&values.get(b)!=null) {if(pairs++!=0)writer.write(",");writer.write("{\"a\":"+a+",\"b\":"+b+",\"equal\":"+values.get(a).equals(values.get(b))+"}");}
            writer.write("]}\n");
        }
        Class.forName("bas").getMethod("close").invoke(ExportItemComponents.resourceManager);System.out.println("original entity tooltip/text cases="+cases.size()+" pairs="+pairs);
    }
}
