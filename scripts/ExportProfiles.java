// Owned observer of original profile codecs, fields and equality.
import com.google.gson.*;
import com.mojang.serialization.*;
import com.mojang.datafixers.util.Either;
import com.mojang.authlib.GameProfile;
import com.mojang.authlib.properties.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;

public final class ExportProfiles {
    static Object field(Object object,String name)throws Exception {return ExportItemComponentSchema.field(object,name);}
    static JsonArray uuid(UUID id) {JsonArray out=new JsonArray();out.add((int)(id.getMostSignificantBits()>>>32));out.add((int)id.getMostSignificantBits());out.add((int)(id.getLeastSignificantBits()>>>32));out.add((int)id.getLeastSignificantBits());return out;}
    static JsonObject describe(Object value)throws Exception {
        JsonObject out=new JsonObject();String cls=value.getClass().getName();out.addProperty("class",cls);
        Either<?,?> contents=(Either<?,?>)field(value,"f");out.addProperty("left",contents.left().isPresent());
        Object body=contents.left().isPresent()?contents.left().get():contents.right().orElseThrow();
        JsonArray properties=new JsonArray();
        if(cls.equals("doy$a")) {
            if(body instanceof String name)out.addProperty("name",name);else out.add("id",uuid((UUID)body));
        } else if(body instanceof GameProfile profile) {
            out.addProperty("name",profile.name());out.add("id",uuid(profile.id()));properties=properties(profile.properties());
        } else {
            Optional<?> name=(Optional<?>)field(body,"c"),id=(Optional<?>)field(body,"d");
            out.add("name",name.isPresent()?new JsonPrimitive((String)name.get()):JsonNull.INSTANCE);
            out.add("id",id.isPresent()?uuid((UUID)id.get()):JsonNull.INSTANCE);
            properties=properties((PropertyMap)field(body,"e"));
        }
        out.add("properties",properties);Object patch=Class.forName("doy").getMethod("c").invoke(value);JsonObject skin=new JsonObject();
        for(String[] f:new String[][]{{"texture","a"},{"cape","b"},{"elytra","c"},{"model","d"}}) {
            Optional<?> entry=(Optional<?>)Class.forName("ddq$a").getMethod(f[1]).invoke(patch);
            if(entry.isEmpty())skin.add(f[0],JsonNull.INSTANCE);
            else if(f[0].equals("model"))skin.addProperty(f[0],(String)Class.forName("ddp").getMethod("c").invoke(entry.get()));
            else {Object texture=entry.get();JsonObject t=new JsonObject();t.addProperty("id",Class.forName("iu$b").getMethod("a").invoke(texture).toString());t.addProperty("path",Class.forName("iu$b").getMethod("b").invoke(texture).toString());skin.add(f[0],t);}
        }
        out.add("skin",skin);return out;
    }
    static JsonArray properties(PropertyMap properties) {
        JsonArray out=new JsonArray();
        for(var entry:properties.asMap().entrySet()) {
            JsonArray group=new JsonArray();for(Property property:entry.getValue()) {
                JsonObject item=new JsonObject();item.addProperty("name",property.name());item.addProperty("value",property.value());
                item.add("signature",property.signature()==null?JsonNull.INSTANCE:new JsonPrimitive(property.signature()));group.add(item);
            }
            JsonObject row=new JsonObject();row.addProperty("key",entry.getKey());row.add("values",group);out.add(row);
        }
        return out;
    }
    @SuppressWarnings("unchecked") public static void main(String[] args)throws Exception {
        Path base=Path.of(args[1]).getParent();
        try(PrintWriter log=new PrintWriter(Files.newBufferedWriter(base.resolve("original-profile-bytecode.log")))) {
            int status=java.util.spi.ToolProvider.findFirst("javap").orElseThrow().run(log,log,"-classpath",System.getProperty("java.class.path"),"-p","-c","doy","doy$a","doy$b","doy$c","ddq$a","ddp","iu$b","bhi","bfm","jx","aam$26","com.mojang.authlib.GameProfile","com.mojang.authlib.properties.PropertyMap");
            if(status!=0)throw new IllegalStateException("owned original inspection failed");
        }
        ExportItemProperties.init("1.21.11");Object stream=Class.forName("doy").getField("b").get(null);
        Codec<Object> codec=(Codec<Object>)Class.forName("doy").getField("a").get(null);
        DynamicOps<Object> ops=(DynamicOps<Object>)Class.forName("vn").getField("a").get(null);
        Method read=Class.forName("vm").getMethod("b",DataInput.class,Class.forName("vi"));Object account=Class.forName("vi").getMethod("a",long.class).invoke(null,16L*1024*1024);
        JsonObject out=new JsonObject(),rules=new JsonObject();JsonArray chars=new JsonArray();
        Method validName=Class.forName("bhi").getMethod("f",String.class);
        for(int c=0;c<=65535;c++)if((boolean)validName.invoke(null,"A"+(char)c+"B"))chars.add(c);
        rules.add("name_utf16",chars);rules.addProperty("empty_name",(boolean)validName.invoke(null,""));
        JsonObject models=new JsonObject();for(boolean bit:new boolean[]{false,true}) {
            Object model=ExportComponentValueRules.decode(Class.forName("ddp").getField("d").get(null),new byte[]{(byte)(bit?1:0)});
            models.addProperty(Boolean.toString(bit),(String)Class.forName("ddp").getMethod("c").invoke(model));
        }
        rules.add("models",models);out.add("rules",rules);out.addProperty("java_version",System.getProperty("java.version"));
        List<Object> values=new ArrayList<>();JsonArray cases=new JsonArray();
        for(JsonElement input:JsonParser.parseString(Files.readString(Path.of(args[0]))).getAsJsonArray()) {
            JsonObject request=input.getAsJsonObject(),row=request.deepCopy();Object value;
            try {
                byte[] bytes=HexFormat.of().parseHex(request.get("input_hex").getAsString());
                if(request.get("format").getAsString().equals("stream"))value=ExportComponentValueRules.decode(stream,bytes);
                else {DataInputStream in=new DataInputStream(new ByteArrayInputStream(bytes));Object tag=read.invoke(null,in,Class.forName("vi").getMethod("a",long.class).invoke(null,16L*1024*1024));if(in.available()!=0)throw new IllegalStateException("owned NBT input has trailing data");value=codec.parse(ops,tag).getOrThrow();}
            } catch(InvocationTargetException|IllegalStateException failure) {
                Throwable original=ExportComponentNormalization.cause(failure);if(original instanceof Error error)throw error;
                row.addProperty("accepted",false);row.addProperty("failure_stage","original_profile_decode");row.addProperty("failure",original.getClass().getName()+": "+original.getMessage());values.add(null);cases.add(row);continue;
            }
            row.addProperty("accepted",true);row.add("fields",describe(value));
            try {
                byte[] canonical=ExportComponentNormalization.encode(stream,value);
                row.addProperty("canonical_wire_hex",HexFormat.of().formatHex(canonical));
                row.addProperty("canonical_wire_equal",value.equals(ExportComponentValueRules.decode(stream,canonical)));
            } catch(InvocationTargetException failure) {
                Throwable original=ExportComponentNormalization.cause(failure);if(original instanceof Error error)throw error;
                row.addProperty("canonical_wire_failure",original.getClass().getName()+": "+original.getMessage());
            }
            values.add(value);cases.add(row);
        }
        JsonArray pairs=new JsonArray();for(int a=0;a<values.size();a++)for(int b=a;b<values.size();b++)if(values.get(a)!=null&&values.get(b)!=null) {
            JsonObject pair=new JsonObject();pair.addProperty("a",a);pair.addProperty("b",b);pair.addProperty("equal",values.get(a).equals(values.get(b)));pairs.add(pair);
        }
        out.add("cases",cases);out.add("pairs",pairs);Files.writeString(Path.of(args[1]),ExportComponentNormalization.jsonUtf16(new GsonBuilder().serializeNulls().setPrettyPrinting().create().toJson(out))+"\n");
        Class.forName("bas").getMethod("close").invoke(ExportItemComponents.resourceManager);
        System.out.println("original profiles="+cases.size()+" pairs="+pairs.size()+" rules="+rules);
    }
}
