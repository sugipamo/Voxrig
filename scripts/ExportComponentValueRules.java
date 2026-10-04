// Own reflection/codec caller. Original enum factories and scalar codecs stay unchanged.
import com.google.gson.*;
import io.netty.buffer.ByteBuf;
import io.netty.buffer.Unpooled;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import java.util.function.*;
import java.math.BigInteger;
import java.nio.ByteBuffer;
import java.io.DataInput;

public final class ExportComponentValueRules {
    static final int[] OUTSIDE={Integer.MIN_VALUE,-65537,-257,-17,-2,-1,256,257,65537,Integer.MAX_VALUE};
    static Object decode(Object codec,byte[] bytes) throws Exception {
        ByteBuf buffer=Unpooled.wrappedBuffer(bytes);
        try {
            Object value=Class.forName("aao").getMethod("decode",Object.class).invoke(codec,ExportItemComponents.buffer(buffer));
            if(buffer.isReadable())throw new IllegalStateException("native scalar codec left bytes");
            return value;
        } finally {buffer.release();}
    }
    static JsonObject scalar(Object value) throws Exception {
        JsonObject out=new JsonObject();
        if(value instanceof Float f) {out.addProperty("kind","float");out.addProperty("bits",Integer.toUnsignedLong(Float.floatToRawIntBits(f)));}
        else if(value instanceof Double d) {out.addProperty("kind","double");out.addProperty("bits",new BigInteger(Long.toUnsignedString(Double.doubleToRawLongBits(d))));}
        else if(value instanceof Integer i) {out.addProperty("kind","integer");out.addProperty("value",i);}
        else if(value instanceof UUID uuid) {out.addProperty("kind","uuid");out.addProperty("most",uuid.getMostSignificantBits());out.addProperty("least",uuid.getLeastSignificantBits());}
        else if(value.getClass().getName().equals("is")) {
            out.addProperty("kind","block_position");for(String[] field:new String[][]{{"x","u"},{"y","v"},{"z","w"}})out.addProperty(field[0],(int)value.getClass().getMethod(field[1]).invoke(value));
        } else throw new IllegalStateException("unreviewed original fixed scalar value "+value.getClass());
        return out;
    }
    @SuppressWarnings("unchecked") public static void main(String[] args) throws Exception {
        Path inspection=Path.of(args[0]).getParent().resolve("by-id-map-bytecode.log");
        try(java.io.PrintWriter log=new java.io.PrintWriter(Files.newBufferedWriter(inspection))) {
            int exit=java.util.spi.ToolProvider.findFirst("javap").orElseThrow().run(log,log,"-classpath",System.getProperty("java.class.path"),"-p","-c","beu");
            if(exit!=0)throw new IllegalStateException("own original enum-factory inspection failed");
        }
        ExportItemProperties.init("1.21.11");
        ExportInventoryTransfers.idOf=ExportItemProperties.id;
        ExportInventoryTransfers.nameOf=ExportItemProperties.name;
        ExportItemComponentSchema.codec=Class.forName("aao");ExportItemComponentSchema.access=ExportItemComponents.registries;
        JsonObject roots=new JsonObject();
        for(Object component:(Iterable<?>)ExportItemProperties.components)
            roots.addProperty(ExportItemProperties.name.invoke(ExportItemProperties.components,component).toString(),ExportItemComponentSchema.node(Class.forName("kh").getMethod("f").invoke(component)));
        roots.addProperty("native_item",ExportItemComponentSchema.node(ExportItemProperties.stack.getField("h").get(null)));
        JsonObject graph=new JsonObject();graph.add("roots",roots);graph.add("nodes",ExportItemComponentSchema.nodes);
        JsonArray enums=new JsonArray(),scalars=new JsonArray();
        List<Map.Entry<Object,Integer>> codecs=new ArrayList<>(ExportItemComponentSchema.ids.entrySet());codecs.sort(Comparator.comparingInt(Map.Entry::getValue));
        for(var entry:codecs) {
            Object codec=entry.getKey();String cls=codec.getClass().getName();int id=entry.getValue();
            if(cls.equals("aam$21")) {
                IntFunction<Object> byId=(IntFunction<Object>)ExportItemComponentSchema.field(codec,"a");
                ToIntFunction<Object> toId=(ToIntFunction<Object>)ExportItemComponentSchema.field(codec,"b");
                Object first=byId.apply(0);
                if(!(first instanceof Enum<?> initial))throw new IllegalStateException("unreviewed native non-enum ID mapper "+id);
                JsonObject descriptor=new JsonObject();descriptor.addProperty("node",id);descriptor.addProperty("value_class",initial.getDeclaringClass().getName());JsonArray values=new JsonArray();
                for(Object value:initial.getDeclaringClass().getEnumConstants()) {
                    JsonObject row=new JsonObject();row.addProperty("name",((Enum<?>)value).name());row.addProperty("native_id",toId.applyAsInt(value));values.add(row);
                    if(Class.forName("bhh").isInstance(value))row.addProperty("serialized_name",(String)Class.forName("bhh").getMethod("c").invoke(value));
                }
                descriptor.add("values",values);JsonArray probes=new JsonArray();TreeSet<Integer> ids=new TreeSet<>();
                for(int value:OUTSIDE)ids.add(value);for(int value=-64;value<=256;value++)ids.add(value);
                for(JsonElement value:values) {int nativeId=value.getAsJsonObject().get("native_id").getAsInt();ids.add(nativeId);ids.add(nativeId-1);ids.add(nativeId+1);}
                for(int value:ids) {
                    Object nativeValue=byId.apply(value);JsonObject probe=new JsonObject();probe.addProperty("input",value);probe.addProperty("native_id",toId.applyAsInt(nativeValue));probe.addProperty("name",((Enum<?>)nativeValue).name());
                    byte[] wire=ExportItemComponents.roundtrip(codec,nativeValue);
                    probe.addProperty("canonical_hex",HexFormat.of().formatHex(wire));probes.add(probe);
                }
                descriptor.add("probes",probes);enums.add(descriptor);
            }
            if(Set.of("aam$2","aam$31","aam$35","is$1","jx$1").contains(cls)) {
                int width=cls.equals("jx$1")?16:Set.of("aam$2","is$1").contains(cls)?8:4;
                Object value=decode(codec,new byte[width]);JsonObject descriptor=new JsonObject();descriptor.addProperty("node",id);descriptor.addProperty("codec_class",cls);descriptor.addProperty("value_class",value.getClass().getName());descriptor.addProperty("width",width);
                descriptor.addProperty("canonical_hex",HexFormat.of().formatHex(ExportItemComponents.roundtrip(codec,value)));scalars.add(descriptor);
                List<byte[]> patterns=new ArrayList<>();
                patterns.add(new byte[width]);byte[] ones=new byte[width];Arrays.fill(ones,(byte)255);patterns.add(ones);
                byte[] sign=new byte[width];sign[0]=(byte)128;patterns.add(sign);
                byte[] sequence=new byte[width];for(int i=0;i<width;i++)sequence[i]=(byte)(i+1);patterns.add(sequence);
                if(cls.equals("aam$35"))for(int bits:new int[]{1,0x3f800000,0x7f800000,0xff800000,0x7fc00000,0x7fc00001,0x7f800001})patterns.add(ByteBuffer.allocate(4).putInt(bits).array());
                if(cls.equals("aam$2"))for(long bits:new long[]{1,0x3ff0000000000000L,0x7ff0000000000000L,0xfff0000000000000L,0x7ff8000000000000L,0x7ff8000000000001L,0x7ff0000000000001L})patterns.add(ByteBuffer.allocate(8).putLong(bits).array());
                JsonArray probes=new JsonArray();for(byte[] pattern:patterns) {
                    Object actual=decode(codec,pattern);JsonObject probe=new JsonObject();probe.addProperty("input_hex",HexFormat.of().formatHex(pattern));probe.add("value",scalar(actual));probe.addProperty("canonical_hex",HexFormat.of().formatHex(ExportItemComponents.roundtrip(codec,actual)));probes.add(probe);
                }
                descriptor.add("probes",probes);
            }
        }
        JsonObject out=new JsonObject();out.add("graph",graph);out.add("enums",enums);out.add("scalars",scalars);
        ExportNbtSemantics.legacy=false;ExportNbtSemantics.io=Class.forName("vm");ExportNbtSemantics.tagClass=Class.forName("vz");ExportNbtSemantics.accountClass=Class.forName("vi");
        ExportNbtSemantics.reader=ExportNbtSemantics.io.getMethod("b",DataInput.class,ExportNbtSemantics.accountClass);
        JsonArray tags=new JsonArray();
        for(String input:new String[]{"00","01ff","028000","0380000000","048000000000000000","0580000000","057fc00001","057f800000","068000000000000000","067ff8000000000001","0700000003ff007f","080003616263","080003eda080","080002c080","09010000000200ff","090a000000020100000500080000000361626300","0a00","0a0300016100000001030001610000000200","0b00000002800000007fffffff","0c0000000280000000000000007fffffffffffffff"}) {
            Object tag=ExportNbtSemantics.read(HexFormat.of().parseHex(input));JsonObject row=new JsonObject();row.addProperty("input_hex",input);
            int kind=((Number)ExportNbtSemantics.tagClass.getMethod("b").invoke(tag)).intValue();
            if(kind==0) {JsonObject end=new JsonObject();end.addProperty("kind",0);row.add("decoded",end);}else row.add("decoded",ExportNbtSemantics.describe(tag));
            Object code=Class.forName("vn").getMethod("a",Class.forName("com.mojang.serialization.DynamicOps"),ExportNbtSemantics.tagClass).invoke(Class.forName("vn").getField("a").get(null),Class.forName("bfw").getField("c").get(null),tag);
            row.addProperty("pure_nbt_crc32c",(int)Class.forName("com.google.common.hash.HashCode").getMethod("asInt").invoke(code));tags.add(row);
        }
        out.add("unnamed_tags",tags);
        Files.writeString(Path.of(args[0]),new GsonBuilder().setPrettyPrinting().create().toJson(out)+"\n");
        System.out.println("original enum nodes="+enums.size()+" fixed scalar nodes="+scalars.size());
        Class.forName("bas").getMethod("close").invoke(ExportItemComponents.resourceManager);
    }
}
