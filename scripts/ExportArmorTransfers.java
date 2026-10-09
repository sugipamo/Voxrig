// Own callers of original armor mayPickup, enchantment constructors and QUICK_MOVE.
// Context supplies actual GameMode fields; no Minecraft algorithm is replaced.
import com.google.gson.*;
import com.mojang.serialization.*;
import java.io.*;
import java.nio.file.*;
import java.util.*;

public final class ExportArmorTransfers {
    static JsonArray snapshot(Object menu) throws Exception {
        JsonArray result = new JsonArray();
        for (Object slot : ExportInventoryTransfers.slots(menu))
            result.add(ExportItemProperties.encoded(ExportInventoryTransfers.slotGet(slot)));
        return result;
    }
    @SuppressWarnings("unchecked") public static void main(String[] args) throws Exception {
        String version = args[0];
        ExportInventoryTransfers.init(version);
        ExportItemProperties.init(version);
        boolean legacy = ExportInventoryTransfers.legacy;
        DynamicOps<JsonElement> ops = legacy ? null : (DynamicOps<JsonElement>) Class.forName("ams")
            .getMethod("a", DynamicOps.class, Class.forName("jf$a"))
            .invoke(null, JsonOps.INSTANCE, ExportItemComponents.registries);
        Map<String, Object> types = new HashMap<>();
        Map<String, Integer> enchantmentIds = new HashMap<>();
        Class<?> component = legacy ? null : Class.forName("kh");
        if (!legacy) for (Object type : (Iterable<?>) ExportItemProperties.components)
            types.put(ExportItemProperties.name.invoke(ExportItemProperties.components, type).toString(), type);
        if (!legacy) {
            java.util.stream.Stream<?> all = (java.util.stream.Stream<?>) Class.forName("jr").getMethod("a").invoke(ExportItemComponents.registries);
            for (Object entry : all.toList()) {
                Object key = Class.forName("jr$d").getMethod("a").invoke(entry);
                if (!Class.forName("amt").getMethod("a").invoke(key).toString().equals("minecraft:enchantment")) continue;
                Object registry = Class.forName("jr$d").getMethod("b").invoke(entry);
                for (Object value : (Iterable<?>) registry)
                    enchantmentIds.put(ExportInventoryTransfers.nameOf.invoke(registry, value).toString(),
                        (Integer) ExportInventoryTransfers.idOf.invoke(registry, value));
            }
        }
        JsonArray cases = new JsonArray();
        for (JsonElement element : new JsonParser().parse(Files.readString(Path.of(args[1]))).getAsJsonArray()) {
            JsonObject request = element.getAsJsonObject();
            ExportInventoryTransfers.reset();
            Object menu = ExportInventoryTransfers.menu("minecraft:player");
            boolean creative = request.get("mode").getAsString().equals("creative");
            Object gameMode = ExportInventoryTransfers.player.getClass()
                .getField(legacy ? "d" : "h").get(ExportInventoryTransfers.player);
            Object gameType = Class.forName(legacy ? "bpy" : "dwl")
                .getField(legacy ? (creative ? "c" : "b") : (creative ? "b" : "a")).get(null);
            ExportInventoryTransfers.field(gameMode, Class.forName(legacy ? "zf" : "axh"), legacy ? "d" : "e", gameType);
            boolean actualCreative = (Boolean) ExportInventoryTransfers.playerClass
                .getMethod(legacy ? "b_" : "ha").invoke(ExportInventoryTransfers.player);
            if (actualCreative != creative) throw new IllegalStateException("actual mode getter differs");
            Object item;
            if (legacy) item = ExportItemProperties.decode(HexFormat.of().parseHex(request.get("input_hex").getAsString()));
            else {
                item = ExportInventoryTransfers.stack(request.get("item").getAsString(), 1);
                for (var patch : request.getAsJsonObject("components").entrySet()) {
                    Object type = Objects.requireNonNull(types.get(patch.getKey()));
                    Object value;
                    if (patch.getKey().equals("minecraft:enchantments") || patch.getKey().equals("minecraft:stored_enchantments")) {
                        // Stream constructors permit level zero, unlike persistent codecs.
                        ByteArrayOutputStream bytes = new ByteArrayOutputStream();
                        DataOutputStream out = new DataOutputStream(bytes);
                        var entries = patch.getValue().getAsJsonObject().entrySet();
                        ExportItemProperties.varint(out, entries.size());
                        for (var enchantment : entries) {
                            ExportItemProperties.varint(out, Objects.requireNonNull(enchantmentIds.get(enchantment.getKey())));
                            ExportItemProperties.varint(out, enchantment.getValue().getAsInt());
                        }
                        value = ExportComponentValueRules.decode(component.getMethod("f").invoke(type), bytes.toByteArray());
                    } else {
                        Codec<?> codec = (Codec<?>) component.getMethod("b").invoke(type);
                        value = codec.parse(ops, patch.getValue()).getOrThrow();
                    }
                    ExportItemProperties.stack.getMethod("b", component, Object.class).invoke(item, type, value);
                }
            }
            int source = request.get("slot").getAsInt();
            List<?> slots = ExportInventoryTransfers.slots(menu);
            ExportInventoryTransfers.slotSet(slots.get(source), item);
            ExportInventoryTransfers.cursor(menu, ExportInventoryTransfers.empty);
            JsonObject row = new JsonObject();
            row.add("request", request);
            row.addProperty("actual_creative", actualCreative);
            row.add("before", snapshot(menu));
            row.addProperty("may_pickup", (Boolean) ExportInventoryTransfers.mayPickup.invoke(slots.get(source), ExportInventoryTransfers.player));
            Object returned = ExportInventoryTransfers.quickMove(menu, source);
            row.add("after", snapshot(menu));
            row.addProperty("cursor_hex", ExportItemProperties.encoded(ExportInventoryTransfers.cursor(menu)));
            if (legacy) row.addProperty("legacy_return_hex", ExportItemProperties.encoded(returned));
            cases.add(row);
        }
        JsonObject output = new JsonObject();
        output.addProperty("version", version);
        output.add("cases", cases);
        if (!legacy) {
            DynamicOps<Object> nbt = (DynamicOps<Object>) Class.forName("ams")
                .getMethod("a", DynamicOps.class, Class.forName("jf$a"))
                .invoke(null, Class.forName("vn").getField("a").get(null), ExportItemComponents.registries);
            Codec<Object> codec = (Codec<Object>) Class.forName("dso").getField("b").get(null);
            JsonArray enchantments = new JsonArray();
            java.util.stream.Stream<?> registries = (java.util.stream.Stream<?>) Class.forName("jr")
                .getMethod("a").invoke(ExportItemComponents.registries);
            for (Object entry : registries.toList()) {
                Object key = Class.forName("jr$d").getMethod("a").invoke(entry);
                String name = Class.forName("amt").getMethod("a").invoke(key).toString();
                if (!name.equals("minecraft:enchantment")) continue;
                Object registry = Class.forName("jr$d").getMethod("b").invoke(entry);
                for (Object value : (Iterable<?>) registry) {
                    JsonObject field = new JsonObject();
                    field.addProperty("name", ExportInventoryTransfers.nameOf.invoke(registry, value).toString());
                    field.addProperty("native_id", (Integer) ExportInventoryTransfers.idOf.invoke(registry, value));
                    Object tag = codec.encodeStart(nbt, value).getOrThrow();
                    ByteArrayOutputStream bytes = new ByteArrayOutputStream();
                    Class.forName("vm").getMethod("a", Class.forName("vz"), DataOutput.class)
                        .invoke(null, tag, new DataOutputStream(bytes));
                    field.addProperty("data_hex", HexFormat.of().formatHex(bytes.toByteArray()));
                    enchantments.add(field);
                }
            }
            output.add("enchantments", enchantments);
            Class.forName("bas").getMethod("close").invoke(ExportItemComponents.resourceManager);
        } else Class.forName("v").getMethod("h").invoke(null);
        Files.writeString(Path.of(args[2]), new GsonBuilder().serializeNulls().create().toJson(output) + "\n");
        System.out.println(version + " original armor pickup/transfer cases=" + cases.size());
    }
}
