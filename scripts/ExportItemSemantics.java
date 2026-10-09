// Own native item equality/persistent hash oracle. Original methods/codecs remain unchanged.
import com.google.gson.*;
import com.google.common.hash.HashCode;
import com.google.common.cache.*;
import com.google.common.util.concurrent.UncheckedExecutionException;
import com.mojang.serialization.*;
import io.netty.buffer.ByteBuf;
import io.netty.buffer.Unpooled;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;

public final class ExportItemSemantics {
    static boolean legacy;
    static Class<?> stack, type, typed;
    static Method matches, sameData, count, empty;
    static Object registryHashOps;
    static final class ItemKey {
        final Object value;
        ItemKey(Object value) {this.value=value;}
        @Override public boolean equals(Object other) {
            try {return other instanceof ItemKey key && (boolean)matches.invoke(null,value,key.value);}
            catch(Exception e) {throw new IllegalStateException(e);}
        }
        @Override public int hashCode() {
            try {
                if((boolean)empty.invoke(value))return 0;
                int hash;
                if(legacy) {
                    Object item=stack.getMethod("b").invoke(value),tag=stack.getMethod("o").invoke(value);
                    hash=31*(int)ExportItemProperties.id.invoke(ExportItemProperties.items,item)+Objects.hashCode(tag);
                } else hash=(int)stack.getMethod("b",stack).invoke(null,value);
                return 31*hash+(int)count.invoke(value);
            } catch(Exception e) {throw new IllegalStateException(e);}
        }
    }
    static Object modernBuffer(ByteBuf bytes) throws Exception {return ExportItemComponents.buffer(bytes);}
    static Object decodeComponent(Object component,byte[] input) throws Exception {
        ByteBuf bytes=Unpooled.wrappedBuffer(input);
        try {
            Object codec=type.getMethod("f").invoke(component);
            Object result=Class.forName("aao").getMethod("decode",Object.class).invoke(codec,modernBuffer(bytes));
            if(bytes.isReadable())throw new IllegalStateException("native component left trailing bytes");
            return result;
        } finally {bytes.release();}
    }
    @SuppressWarnings("unchecked") static DataResult<HashCode> encode(Object component,Object value,Object ops) throws Exception {
        Object input=typed.getConstructor(type,Object.class).newInstance(component,value);
        return (DataResult<HashCode>)typed.getMethod("a",DynamicOps.class).invoke(input,ops);
    }
    static Object withRegistries(DynamicOps<HashCode> ops) throws Exception {
        return Class.forName("ams").getMethod("a",DynamicOps.class,Class.forName("jf$a")).invoke(null,ops,ExportItemComponents.registries);
    }
    static String failure(Throwable failure) {
        while(failure instanceof InvocationTargetException || failure instanceof java.util.concurrent.ExecutionException || failure instanceof UncheckedExecutionException)failure=failure.getCause();
        return failure.getClass().getName()+": "+failure.getMessage();
    }
    static void rejectRecorderFailure(Throwable failure) {
        for(Throwable cause=failure;cause!=null;cause=cause.getCause())
            if(cause instanceof IllegalStateException && cause.getMessage()!=null && cause.getMessage().startsWith("own recorder"))
                throw new IllegalStateException("own recorder failed during native hashed-stack composition",cause);
    }
    static boolean sameItemData(Object a,Object b) throws Exception {
        return (boolean)sameData.invoke(null,a,b) && (!legacy || (boolean)stack.getMethod("a",stack,stack).invoke(null,a,b));
    }
    static JsonArray prototypeNeutrality(JsonArray requests) throws Exception {
        JsonArray result=new JsonArray();
        for(JsonElement element:requests) {
            JsonObject request=element.getAsJsonObject();byte[] original=HexFormat.of().parseHex(request.get("base_hex").getAsString());
            byte[] changed=HexFormat.of().parseHex(request.get("changed_hex").getAsString());
            Object a=ExportItemProperties.decode(original),b=ExportItemProperties.decode(changed);
            JsonObject row=new JsonParser().parse(request.toString()).getAsJsonObject();row.addProperty("same_item_data",sameItemData(a,b));row.addProperty("matches",(boolean)matches.invoke(null,a,b));
            row.addProperty("canonical_changed_hex",ExportItemProperties.encoded(b));result.add(row);
        }
        return result;
    }
    @SuppressWarnings("unchecked") public static void main(String[] args) throws Exception {
        ExportItemProperties.init(args[0]);legacy=ExportItemProperties.legacy;stack=ExportItemProperties.stack;
        count=stack.getMethod(legacy?"E":"N");empty=stack.getMethod(legacy?"a":"f");
        matches=stack.getMethod(legacy?"b":"a",stack,stack);sameData=stack.getMethod("c",stack,stack);
        JsonObject requests=new JsonParser().parse(Files.readString(Path.of(args[1]))).getAsJsonObject();
        JsonObject output=new JsonObject();output.addProperty("version",args[0]);
        JsonArray items=new JsonArray();Map<ItemKey,Integer> itemGroups=new HashMap<>();
        for(JsonElement element:requests.getAsJsonArray("items")) {
            JsonObject request=element.getAsJsonObject();byte[] bytes=HexFormat.of().parseHex(request.get("input_hex").getAsString());
            Object value=ExportItemProperties.decode(bytes),independent=ExportItemProperties.decode(bytes);
            ItemKey key=new ItemKey(value);Integer group=itemGroups.get(key);if(group==null){group=itemGroups.size();itemGroups.put(key,group);}
            JsonObject row=new JsonParser().parse(request.toString()).getAsJsonObject();row.addProperty("group",group);row.addProperty("independent_decode_matches",(boolean)matches.invoke(null,value,independent));
            row.addProperty("canonical_hex",ExportItemProperties.encoded(value));items.add(row);
        }
        output.add("items",items);output.addProperty("item_equivalence_groups",itemGroups.size());
        output.add("prototype_cases",prototypeNeutrality(requests.getAsJsonArray("prototype_cases")));
        if(!legacy) {
            type=Class.forName("kh");typed=Class.forName("kk");
            Map<String,Object> types=new HashMap<>();for(Object component:(Iterable<?>)ExportItemProperties.components)types.put(ExportItemProperties.name.invoke(ExportItemProperties.components,component).toString(),component);
            RecordingNativeHashOps.Graph graph=new RecordingNativeHashOps.Graph();RecordingNativeHashOps recording=new RecordingNativeHashOps(graph);
            registryHashOps=withRegistries(recording.ops());
            Object plainOps=withRegistries((DynamicOps<HashCode>)Class.forName("bfw").getField("c").get(null));
            Map<String,Map<Object,Integer>> groups=new HashMap<>();JsonArray components=new JsonArray();
            Map<Object,Integer> cachedRoots=new HashMap<>();
            LoadingCache<Object,Integer> cache=CacheBuilder.newBuilder().maximumSize(256).build(new CacheLoader<>() {
                @Override public Integer load(Object input) throws Exception {
                    DataResult<HashCode> encoded=(DataResult<HashCode>)typed.getMethod("a",DynamicOps.class).invoke(input,registryHashOps);
                    HashCode code=encoded.getOrThrow();cachedRoots.put(input,recording.node(code));return code.asInt();
                }
            });
            // Own required HashGenerator callback delegates to original typed encoders and
            // original Guava cache. This is a primitive/codec composition, not a live server cache.
            Class<?> generator=Class.forName("wz$a");Object hashGenerator=Proxy.newProxyInstance(generator.getClassLoader(),new Class<?>[]{generator},(p,m,a)->cache.getUnchecked(a[0]));
            for(JsonElement element:requests.getAsJsonArray("components")) {
                JsonObject request=element.getAsJsonObject();String name=request.get("component").getAsString();Object component=types.get(name);
                if(component==null)throw new IllegalStateException("unknown requested native component");
                byte[] input=HexFormat.of().parseHex(request.get("input_hex").getAsString());Object value=decodeComponent(component,input),independent=decodeComponent(component,input);
                Map<Object,Integer> componentGroups=groups.computeIfAbsent(name,k->new HashMap<>());Integer group=componentGroups.get(value);
                if(group==null){group=componentGroups.size();componentGroups.put(value,group);}
                JsonObject row=new JsonParser().parse(request.toString()).getAsJsonObject();row.addProperty("native_id",(int)ExportItemProperties.id.invoke(ExportItemProperties.components,component));row.addProperty("value_class",value.getClass().getName());row.addProperty("group",group);row.addProperty("independent_decode_equal",value.equals(independent));
                row.addProperty("canonical_hex",HexFormat.of().formatHex(ExportItemComponents.roundtrip(type.getMethod("f").invoke(component),value)));
                Object persistent=type.getMethod("b").invoke(component);row.addProperty("persistent_codec_class",persistent==null?null:persistent.getClass().getName());
                DataResult<HashCode> actual=encode(component,value,plainOps),observed=encode(component,value,registryHashOps);
                if(actual.result().isPresent()!=observed.result().isPresent())throw new IllegalStateException("observed/native encoder success differs: "+name);
                if(actual.result().isPresent()) {
                    HashCode code=actual.result().orElseThrow(),observedCode=observed.result().orElseThrow();
                    if(!code.equals(observedCode))throw new IllegalStateException("observed/native hash differs: "+name);
                    row.addProperty("persistent_crc32c",code.asInt());row.addProperty("hash_node",recording.node(observedCode));
                    if(request.has("fresh_hash")) {
                        RecordingNativeHashOps fresh=new RecordingNativeHashOps(graph);DataResult<HashCode> uncached=encode(component,value,withRegistries(fresh.ops()));
                        HashCode freshCode=uncached.getOrThrow();row.addProperty("fresh_persistent_crc32c",freshCode.asInt());row.addProperty("fresh_hash_node",fresh.node(freshCode));
                    }
                    Object typedValue=typed.getConstructor(type,Object.class).newInstance(component,value);
                    row.addProperty("outer_cached_crc32c",cache.getUnchecked(typedValue));row.addProperty("outer_cached_hash_node",cachedRoots.get(typedValue));
                } else row.addProperty("native_encoder_error",actual.error().orElseThrow().message());
                // Run the original full hashed-stack creator and stream codec on a real
                // native decoded stone stack with this component, not a patched game method.
                byte[] item=HexFormat.of().parseHex(request.get("item_hex").getAsString());Object itemValue=ExportItemProperties.decode(item);
                try {
                    Class<?> hash=Class.forName("xa");Object hashed=hash.getMethod("b",stack,generator).invoke(null,itemValue,hashGenerator);
                    if(!(boolean)hash.getMethod("a",stack,generator).invoke(hashed,itemValue,hashGenerator))throw new IllegalStateException("original hashed stack does not match its source");
                    row.addProperty("hashed_stack_hex",HexFormat.of().formatHex(ExportItemComponents.roundtrip(hash.getField("b").get(null),hashed)));
                } catch(InvocationTargetException failure) {rejectRecorderFailure(failure);row.addProperty("native_hashed_stack_error",failure(failure));}
                components.add(row);
            }
            output.add("components",components);output.add("hash_nodes",graph.nodes);
            JsonObject groupCounts=new JsonObject();for(var entry:new TreeMap<>(groups).entrySet())groupCounts.addProperty(entry.getKey(),entry.getValue().size());output.add("component_equivalence_groups",groupCounts);
            output.addProperty("outer_cache_maximum",256);output.addProperty("outer_cache_scope","Own required HashGenerator composition of original typed persistent encoding and original Guava native-key cache, matching mapped primitive capacity. Not live ServerPlayer synchronizer/cache proof.");
        }
        Files.writeString(Path.of(args[2]),new GsonBuilder().setPrettyPrinting().serializeNulls().create().toJson(output)+"\n");
        System.out.println(args[0]+" item rows="+items.size()+" groups="+itemGroups.size()+" prototype cases="+output.getAsJsonArray("prototype_cases").size());
        if(ExportItemComponents.resourceManager!=null)Class.forName("bas").getMethod("close").invoke(ExportItemComponents.resourceManager);
    }
}
