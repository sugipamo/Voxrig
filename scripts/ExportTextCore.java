// Own scalar/text-field observer; original game/JDK codecs and constructors remain unchanged.
import com.google.gson.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import java.io.*;

public final class ExportTextCore {
    static Object field(Object value,String name)throws Exception {return ExportItemComponentSchema.field(value,name);}
    static String text(Object value)throws Exception {return value.toString();}
    static JsonElement describeArg(Object value)throws Exception {
        JsonObject out=new JsonObject();out.addProperty("class",value.getClass().getName());
        if(value instanceof String s)out.addProperty("string",s);
        else if(value instanceof Double d)out.addProperty("double_bits",Long.toUnsignedString(Double.doubleToRawLongBits(d)));
        else if(value instanceof Float f)out.addProperty("float_bits",Integer.toUnsignedLong(Float.floatToRawIntBits(f)));
        else if(value instanceof Number n)out.addProperty("integer_value",n.longValue());
        else if(value instanceof Boolean b)out.addProperty("boolean",b);
        else if(Class.forName("yh").isInstance(value))out.add("text",describe(value));
        else throw new IllegalStateException("own unreviewed native translation argument "+value.getClass());
        return out;
    }
    static JsonObject describe(Object value)throws Exception {
        JsonObject out=new JsonObject();Object contents=Class.forName("yh").getMethod("b").invoke(value),style=Class.forName("yh").getMethod("a").invoke(value);
        String cls=contents.getClass().getName();out.addProperty("contents_class",cls);JsonObject body=new JsonObject();
        switch(cls) {
            case "zn$1","zn$a" -> body.addProperty("text",(String)Class.forName("zn").getMethod("b").invoke(contents));
            case "zj" -> body.addProperty("keybind",(String)field(contents,"b"));
            case "zq" -> {
                body.addProperty("key",(String)field(contents,"g"));body.add("fallback",new GsonBuilder().serializeNulls().create().toJsonTree(field(contents,"h")));
                JsonArray args=new JsonArray();for(Object arg:(Object[])field(contents,"i"))args.add(describeArg(arg));body.add("arguments",args);
            }
            case "zp" -> {body.addProperty("pattern",(String)Class.forName("gx").getMethod("a").invoke(field(contents,"b")));Object separator=((Optional<?>)field(contents,"c")).orElse(null);body.add("separator",separator==null?JsonNull.INSTANCE:describe(separator));}
            case "zo" -> {
                Object name=field(contents,"c");var either=(com.mojang.datafixers.util.Either<?,?>)name;
                body.addProperty("name_is_selector",either.left().isPresent());Object actual=either.left().isPresent()?either.left().get():either.right().orElseThrow();
                body.addProperty("name",actual instanceof String s?s:(String)Class.forName("gx").getMethod("a").invoke(actual));body.addProperty("objective",(String)field(contents,"d"));
            }
            case "zl" -> {
                body.addProperty("path",(String)field(contents,"f"));body.addProperty("interpret",(boolean)field(contents,"d"));
                Object separator=((Optional<?>)field(contents,"e")).orElse(null);body.add("separator",separator==null?JsonNull.INSTANCE:describe(separator));
                Object source=field(contents,"g");body.addProperty("source_class",source.getClass().getName());body.addProperty("source",text(field(source,"b")));
            }
            case "zm" -> {
                Object object=field(contents,"b");body.addProperty("object_class",object.getClass().getName());
                if(object.getClass().getName().equals("zy")) {body.addProperty("atlas",text(field(object,"c")));body.addProperty("sprite",text(field(object,"d")));}
                else if(object.getClass().getName().equals("aab")) {body.addProperty("profile_class",field(object,"b").getClass().getName());body.addProperty("hat",(boolean)field(object,"c"));}
                else throw new IllegalStateException("own unreviewed native text object "+object.getClass());
            }
            default -> throw new IllegalStateException("own unreviewed native contents "+cls);
        }
        out.add("body",body);JsonObject s=new JsonObject();
        for(String[] f:new String[][]{{"bold","e"},{"italic","f"},{"underlined","g"},{"strikethrough","h"},{"obfuscated","i"},{"shadow_color","d"},{"insertion","l"}})
            s.add(f[0],new GsonBuilder().serializeNulls().create().toJsonTree(field(style,f[1])));
        Object color=field(style,"c");
        if(color==null)s.add("color",JsonNull.INSTANCE);else {
            JsonObject c=new JsonObject();c.addProperty("rgb",(int)Class.forName("zh").getMethod("a").invoke(color));c.addProperty("serialized",(String)Class.forName("zh").getMethod("b").invoke(color));s.add("color",c);
        }
        Object font=field(style,"m");s.add("font",font==null?JsonNull.INSTANCE:new JsonPrimitive(text(Class.forName("ym$c").getMethod("a").invoke(font))));
        for(String[] f:new String[][]{{"click_class","j"},{"hover_class","k"}}) {Object v=field(style,f[1]);s.add(f[0],v==null?JsonNull.INSTANCE:new JsonPrimitive(v.getClass().getName()));}
        out.add("style",s);JsonArray siblings=new JsonArray();for(Object child:(List<?>)Class.forName("yh").getMethod("c").invoke(value))siblings.add(describe(child));out.add("siblings",siblings);return out;
    }
    public static void main(String[] args)throws Exception {
        try(PrintWriter log=new PrintWriter(Files.newBufferedWriter(Path.of(args[1]).getParent().resolve("original-text-numeric-bytecode.log")))) {
            int status=java.util.spi.ToolProvider.findFirst("javap").orElseThrow().run(log,log,"-classpath",System.getProperty("java.class.path"),"-p","-c","bel","bfm","bgj","vn","yj","zq");
            if(status!=0)throw new IllegalStateException("own native numeric inspection failed");
        }
        ExportItemProperties.init("1.21.11");Object type=null;
        for(Object candidate:(Iterable<?>)ExportItemProperties.components)if(ExportItemProperties.name.invoke(ExportItemProperties.components,candidate).toString().equals("minecraft:custom_name"))type=candidate;
        Object codec=Class.forName("kh").getMethod("f").invoke(type);JsonObject out=new JsonObject();out.addProperty("java_version",System.getProperty("java.version"));JsonObject colors=new JsonObject();
        Field colorNames=Class.forName("zh").getDeclaredField("d");colorNames.setAccessible(true);
        for(var entry:((Map<?,?>)colorNames.get(null)).entrySet())colors.addProperty(entry.getKey().toString(),(int)Class.forName("zh").getMethod("a").invoke(entry.getValue()));out.add("colors",colors);
        JsonObject digits=new JsonObject();for(int c=0;c<=Character.MAX_VALUE;c++) {int digit=Character.digit((char)c,16);if(digit>=0)digits.addProperty(Integer.toString(c),digit);}out.add("hex_utf16_digits",digits);
        JsonArray cases=new JsonArray();List<Object> values=new ArrayList<>();for(JsonElement element:JsonParser.parseString(Files.readString(Path.of(args[0]))).getAsJsonArray()) {
            JsonObject request=element.getAsJsonObject();JsonObject row=new JsonObject();row.addProperty("case",request.get("case").getAsString());row.addProperty("input_hex",request.get("input_hex").getAsString());
            try {
                Object value=ExportComponentValueRules.decode(codec,HexFormat.of().parseHex(row.get("input_hex").getAsString()));
                row.addProperty("accepted",true);row.add("fields",describe(value));values.add(value);
            }catch(InvocationTargetException failure) {
                Throwable original=ExportComponentNormalization.cause(failure);if(original instanceof Error error)throw error;
                row.addProperty("accepted",false);row.addProperty("failure",original.getClass().getName()+": "+original.getMessage());values.add(null);
            }
            cases.add(row);
        }
        JsonArray pairs=new JsonArray();for(int a=0;a<values.size();a++)for(int b=a;b<values.size();b++)if(values.get(a)!=null&&values.get(b)!=null) {
            JsonObject pair=new JsonObject();pair.addProperty("a",a);pair.addProperty("b",b);pair.addProperty("equal",values.get(a).equals(values.get(b)));pairs.add(pair);
        }
        out.add("cases",cases);out.add("pairs",pairs);Files.writeString(Path.of(args[1]),ExportComponentNormalization.jsonUtf16(new GsonBuilder().serializeNulls().setPrettyPrinting().create().toJson(out))+"\n");
        System.out.println("original text fields="+cases.size()+" colors="+colors.size()+" UTF16 hex digits="+digits.size());
        Class.forName("bas").getMethod("close").invoke(ExportItemComponents.resourceManager);
    }
}
