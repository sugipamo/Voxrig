// Own observer of unchanged holder/holder-set constructors, registry lookups,
// registry-aware stream decoders and equals. No game methods are replaced.
import com.google.gson.*;
import com.mojang.serialization.Lifecycle;
import io.netty.buffer.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import java.io.*;

public final class ExportHolderComparison {
    static Class<?> key, identifier, mapped, holder, tag, access, codec;
    static Object registryKey;
    static List<Object> values = new ArrayList<>();
    static JsonArray cases = new JsonArray();
    static IdentityHashMap<Object, Integer> identities = new IdentityHashMap<>();
    static Method tagMethod(String name, Class<?>... parameters) throws Exception { Method method = mapped.getDeclaredMethod(name, parameters); method.setAccessible(true); return method; }
    static Object id(String name) throws Exception { return identifier.getMethod("a", String.class).invoke(null, name); }
    static Object entryKey(String name) throws Exception { return key.getMethod("a", key, identifier).invoke(null, registryKey, id(name)); }
    static Object tagKey(String name) throws Exception { return tag.getMethod("a", key, identifier).invoke(null, registryKey, id(name)); }
    static Object reference(Object registry, int id) throws Exception { return ((Optional<?>) mapped.getMethod("c", int.class).invoke(registry, id)).orElseThrow(); }
    static void add(String name, String kind, int owner, Object value) throws Exception {
        JsonObject row = new JsonObject();
        row.addProperty("case", name); row.addProperty("kind", kind); row.addProperty("owner", owner);
        row.addProperty("identity", identities.computeIfAbsent(value, ignored -> identities.size()));
        row.addProperty("class", value.getClass().getName());
        if (kind.equals("reference")) {
            row.addProperty("entry", holder.getMethod("g").invoke(value).toString());
        } else if (kind.equals("named") || kind.equals("direct_set")) {
            JsonArray members = new JsonArray();
            try {
                for (Object member : ((java.util.stream.Stream<?>) Class.forName("jh").getMethod("a").invoke(value)).toList())
                    members.add(holder.getMethod("g").invoke(member).toString());
                row.add("members", members);
            } catch (InvocationTargetException error) {
                row.addProperty("members_failure", error.getCause().getClass().getName());
            }
            if (kind.equals("named")) row.addProperty("tag", Class.forName("jh$c").getMethod("h").invoke(value).toString());
        }
        values.add(value); cases.add(row);
    }
    static Object decode(Object stream, Object registries, byte[] input) throws Exception {
        ByteBuf bytes = Unpooled.wrappedBuffer(input);
        try {
            Object buffer = Class.forName("xq").getConstructor(ByteBuf.class, access).newInstance(bytes, registries);
            Object value = codec.getMethod("decode", Object.class).invoke(stream, buffer);
            if (bytes.isReadable()) throw new IllegalStateException("original decoder left bytes");
            return value;
        } finally { bytes.release(); }
    }
    public static void main(String[] args) throws Exception {
        ExportItemProperties.init("1.21.11");
        key = Class.forName("amt"); identifier = Class.forName("amo"); mapped = Class.forName("jl");
        holder = Class.forName("jd"); tag = Class.forName("bef"); access = Class.forName("jr"); codec = Class.forName("aao");
        Object original = null;
        for (Object entry : ((java.util.stream.Stream<?>) access.getMethod("a").invoke(ExportItemComponents.registries)).toList()) {
            Object registry = Class.forName("jr$d").getMethod("b").invoke(entry);
            Object k = Class.forName("jq").getMethod("g").invoke(registry);
            if (k.toString().contains(" / minecraft:enchantment]")) { original = registry; registryKey = k; break; }
        }
        if (original == null) throw new IllegalStateException("original enchantment registry missing");
        List<Object> originalValues = new ArrayList<>();
        for (Object value : (Iterable<?>) original) { originalValues.add(value); if (originalValues.size() == 2) break; }
        Object registration = Class.forName("jp").getField("a").get(null);
        Object[] registries = new Object[2], contexts = new Object[2];
        for (int owner = 0; owner < 2; owner++) {
            Object registry = mapped.getConstructor(key, Lifecycle.class).newInstance(registryKey, Lifecycle.stable());
            registries[owner] = registry;
            for (int entry = 0; entry < 2; entry++) {
                int index = owner == 0 ? entry : 1 - entry;
                String name = "example:" + (index == 0 ? "first" : "second");
                mapped.getMethod("a", key, Object.class, Class.forName("jp")).invoke(registry, entryKey(name), originalValues.get(index), registration);
            }
            for (String name : List.of("example:tag", "example:alias")) {
                Object tk = tagKey(name);
                tagMethod("d", tag).invoke(registry, tk);
                tagMethod("a", tag, List.class).invoke(registry, tk, List.of(reference(registry, 0)));
            }
            mapped.getMethod("n").invoke(registry);
            Object entry = Class.forName("jr$d").getConstructor(key, Class.forName("jq")).newInstance(registryKey, registry);
            contexts[owner] = Class.forName("jr$c").getConstructor(java.util.stream.Stream.class).newInstance(java.util.stream.Stream.of(entry));
            for (int entryId : new int[]{0, 1, 0}) add("lookup-"+owner+"-"+entryId, "reference", owner, reference(registry, entryId));
            for (String name : List.of("example:tag", "example:alias", "example:tag"))
                add("tag-lookup-"+owner+"-"+name, "named", owner, ((Optional<?>) mapped.getMethod("a", tag).invoke(registry, tagKey(name))).orElseThrow());
            for (List<Integer> ids : List.of(List.<Integer>of(), List.of(0), List.of(0,1), List.of(1,0), List.of(0,0), List.of(0))) {
                List<Object> members = new ArrayList<>(); for (int entryId : ids) members.add(reference(registry, entryId));
                add("direct-set-"+owner+"-"+ids, "direct_set", owner, Class.forName("jh").getMethod("a", List.class).invoke(null, members));
            }
            for (int index : new int[]{0, 1, 0}) add("direct-value-"+owner+"-"+index, "direct", owner, holder.getMethod("a", Object.class).invoke(null, originalValues.get(index)));
            for (int repeat = 0; repeat < 2; repeat++) {
                Object value = Class.forName("jh").getMethod("a", Class.forName("jg"), tag).invoke(null, registry, tagKey("example:tag"));
                add("independent-empty-named-"+owner+"-"+repeat, "named", owner, value);
            }
        }
        JsonArray streamCases = new JsonArray(), streamFailures = new JsonArray();
        Object references = Class.forName("aam").getMethod("b", key).invoke(null, registryKey);
        Object sets = Class.forName("aam").getMethod("c", key).invoke(null, registryKey);
        for (int owner = 0; owner < 2; owner++) for (int repeat = 0; repeat < 2; repeat++) {
            add("reference-stream-"+owner+"-"+repeat, "reference", owner, decode(references, contexts[owner], new byte[]{0}));
            for (String name : List.of("example:tag", "example:missing")) {
                ByteArrayOutputStream bytes = new ByteArrayOutputStream(); bytes.write(0);
                byte[] text = name.getBytes(java.nio.charset.StandardCharsets.UTF_8); bytes.write(text.length); bytes.write(text);
                JsonObject row = new JsonObject();row.addProperty("owner",owner);row.addProperty("repeat",repeat);row.addProperty("input_hex", HexFormat.of().formatHex(bytes.toByteArray()));
                try {
                    Object value = decode(sets, contexts[owner], bytes.toByteArray());
                    add("tag-stream-"+owner+"-"+repeat+"-"+name, "named", owner, value);
                    row.addProperty("case_index", cases.size()-1);streamCases.add(row);
                } catch (InvocationTargetException error) {
                    row.addProperty("failure",error.getCause().getClass().getName());streamFailures.add(row);
                }
            }
        }
        JsonArray pairs = new JsonArray();
        for (int a=0;a<values.size();a++) for(int b=a;b<values.size();b++) {
            JsonObject pair=new JsonObject();pair.addProperty("a",a);pair.addProperty("b",b);pair.addProperty("equal",values.get(a).equals(values.get(b)));pairs.add(pair);
        }
        JsonArray nbtBoundaries = new JsonArray();
        for (Object type : (Iterable<?>) ExportItemProperties.components) {
            String name = ExportItemProperties.name.invoke(ExportItemProperties.components, type).toString();
            if (!Set.of("minecraft:custom_data", "minecraft:bucket_entity_data", "minecraft:entity_data", "minecraft:block_entity_data").contains(name)) continue;
            Object stream = Class.forName("kh").getMethod("f").invoke(type);
            boolean typed = name.equals("minecraft:entity_data") || name.equals("minecraft:block_entity_data");
            for (String nbt : List.of("00", "0300000001", "0a00")) {
                String input = (typed ? "00" : "") + nbt;
                JsonObject row = new JsonObject(); row.addProperty("component", name); row.addProperty("input_hex", input);
                try {
                    Object value = decode(stream, ExportItemComponents.registries, HexFormat.of().parseHex(input));
                    row.addProperty("accepted", true); row.addProperty("class", value.getClass().getName());
                } catch (InvocationTargetException error) {
                    row.addProperty("accepted", false); row.addProperty("failure", error.getCause().getClass().getName());
                }
                nbtBoundaries.add(row);
            }
        }
        JsonObject out=new JsonObject();out.addProperty("java_version",System.getProperty("java.version"));out.add("cases",cases);out.add("pairs",pairs);out.add("stream_cases",streamCases);out.add("stream_failures",streamFailures);out.add("nbt_boundaries",nbtBoundaries);
        Files.writeString(Path.of(args[0]),new GsonBuilder().serializeNulls().create().toJson(out));
        Class.forName("bas").getMethod("close").invoke(ExportItemComponents.resourceManager);
        System.out.println("original holder comparison cases="+cases.size()+" pairs="+pairs.size());
    }
}
