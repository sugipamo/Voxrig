// Own observer of original builtin registry identity, IDs and registry synchronization.
import com.google.gson.*;
import com.mojang.serialization.*;
import java.nio.file.*;
import java.util.*;
import java.io.*;

public final class ExportRegistryCatalog {
    static Object result(Object result) throws Exception {
        return ((Optional<?>) Class.forName("com.mojang.serialization.DataResult")
            .getMethod("result").invoke(result)).orElseThrow();
    }
    static Object encode(Object codec, Object ops, Object value) throws Exception {
        // DataResult is a class in the legacy bundle and an interface in modern.
        Object result = Class.forName("com.mojang.serialization.Encoder")
            .getMethod("encodeStart", DynamicOps.class, Object.class).invoke(codec, ops, value);
        return result(result);
    }
    static JsonArray entries(Object registry) throws Exception {
        JsonArray out = new JsonArray();
        for (Object value : (Iterable<?>) registry) {
            JsonObject row = new JsonObject();
            row.addProperty("name", ExportItemProperties.name.invoke(registry, value).toString());
            row.addProperty("id", (Integer) ExportItemProperties.id.invoke(registry, value));
            out.add(row);
        }
        return out;
    }
    static String key(Object key) { return key.toString().split(" / ")[1].replace("]", ""); }
    static JsonObject copy(JsonElement value) { return new Gson().fromJson(value.toString(), JsonObject.class); }
    public static void main(String[] args) throws Exception {
        String version = args[0];
        ExportItemProperties.init(version);
        boolean legacy = version.equals("1.16.1");
        Object root = Class.forName(legacy ? "gl" : "mi").getField(legacy ? "h" : "aR").get(null);
        Set<Object> builtin = Collections.newSetFromMap(new IdentityHashMap<>());
        JsonArray rows = new JsonArray();
        for (Object registry : (Iterable<?>) root) {
            builtin.add(registry);
            JsonObject row = new JsonObject();
            row.addProperty("name", ExportItemProperties.name.invoke(root, registry).toString());
            row.addProperty("builtin", true);
            row.addProperty("networkable", false);
            row.add("entries", entries(registry));
            rows.add(row);
        }
        JsonObject out = new JsonObject();
        if (legacy) {
            Object holder = Class.forName("gm").getMethod("b").invoke(null);
            var dimensionGetter = Class.forName("gm$a").getMethod("a");
            Object dimensions = dimensionGetter.invoke(holder);
            JsonObject row = new JsonObject();
            row.addProperty("name", "minecraft:dimension_type");
            row.addProperty("builtin", false);
            row.addProperty("networkable", true);
            row.add("entries", entries(dimensions));
            rows.add(row);
            Object codec = Class.forName("gm$a").getField("a").get(null);
            out.add("legacy_builtin_codec_json", (JsonElement) encode(codec, JsonOps.INSTANCE, holder));
            @SuppressWarnings("unchecked") DynamicOps<Object> ops = (DynamicOps<Object>) Class.forName("lp").getField("a").get(null);
            Object nbt = encode(codec, ops, holder);
            ByteArrayOutputStream bytes = new ByteArrayOutputStream();
            Class.forName("lo").getMethod("a", Class.forName("le"), DataOutput.class).invoke(null, nbt, new DataOutputStream(bytes));
            out.addProperty("legacy_builtin_codec_hex", HexFormat.of().formatHex(bytes.toByteArray()));
            JsonArray cases = new JsonArray();
            JsonObject original = out.getAsJsonObject("legacy_builtin_codec_json");
            for (String variant : List.of("reversed", "empty", "subset", "custom_names")) {
                JsonObject input = copy(original);
                JsonArray source = original.getAsJsonArray("dimension"), changed = new JsonArray();
                if (variant.equals("reversed")) for (int i = source.size() - 1; i >= 0; i--) changed.add(copy(source.get(i)));
                else if (!variant.equals("empty")) for (int i = 0; i < 2; i++) {
                    JsonObject value = copy(source.get(i));
                    if (variant.equals("custom_names")) value.addProperty("name", "example:dimension_" + i);
                    changed.add(value);
                }
                input.add("dimension", changed);
                Object inputNbt = JsonOps.INSTANCE.convertTo(ops, input);
                Object decoded = result(Class.forName("com.mojang.serialization.Decoder")
                    .getMethod("parse", DynamicOps.class, Object.class).invoke(codec, ops, inputNbt));
                ByteArrayOutputStream encoded = new ByteArrayOutputStream();
                Class.forName("lo").getMethod("a", Class.forName("le"), DataOutput.class).invoke(null, inputNbt, new DataOutputStream(encoded));
                JsonObject sample = new JsonObject();
                sample.addProperty("case", variant);
                sample.addProperty("codec_hex", HexFormat.of().formatHex(encoded.toByteArray()));
                sample.add("entries", entries(dimensionGetter.invoke(decoded)));
                cases.add(sample);
            }
            out.add("legacy_codec_cases", cases);
        } else {
            var method = Class.forName("ju").getDeclaredMethod("a", Class.forName("jr"));
            method.setAccessible(true);
            Set<String> networked = new HashSet<>();
            for (Object entry : ((java.util.stream.Stream<?>) method.invoke(null, ExportItemComponents.registries)).toList())
                networked.add(key(Class.forName("jr$d").getMethod("a").invoke(entry)));
            for (Object entry : ((java.util.stream.Stream<?>) Class.forName("jr").getMethod("a").invoke(ExportItemComponents.registries)).toList()) {
                Object registry = Class.forName("jr$d").getMethod("b").invoke(entry);
                if (builtin.contains(registry)) continue;
                String name = key(Class.forName("jr$d").getMethod("a").invoke(entry));
                JsonObject row = new JsonObject();
                row.addProperty("name", name);
                row.addProperty("builtin", false);
                row.addProperty("networkable", networked.contains(name));
                row.add("entries", entries(registry));
                rows.add(row);
            }
            Class.forName("bas").getMethod("close").invoke(ExportItemComponents.resourceManager);
        }
        out.addProperty("version", version);
        out.addProperty("java_version", System.getProperty("java.version"));
        out.add("registries", rows);
        Files.writeString(Path.of(args[1]), new GsonBuilder().serializeNulls().create().toJson(out));
        System.out.println("actual registry catalog " + version + " registries=" + rows.size());
    }
}
