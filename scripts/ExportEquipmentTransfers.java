// Own observer of unchanged modern equipment routing and actual menu QUICK_MOVE.
// Uses original component constructors, slot getters and setters; no game bodies.
import com.google.gson.*;
import com.mojang.serialization.*;
import java.nio.file.*;
import java.util.*;

public final class ExportEquipmentTransfers {
    static JsonArray snapshot(Object menu) throws Exception {
        JsonArray out = new JsonArray();
        for (Object slot : ExportInventoryTransfers.slots(menu))
            out.add(ExportItemProperties.encoded(ExportInventoryTransfers.slotGet(slot)));
        return out;
    }
    @SuppressWarnings("unchecked") public static void main(String[] args) throws Exception {
        ExportInventoryTransfers.init("1.21.11");
        ExportItemProperties.init("1.21.11");
        DynamicOps<JsonElement> ops = (DynamicOps<JsonElement>) Class.forName("ams")
            .getMethod("a", DynamicOps.class, Class.forName("jf$a"))
            .invoke(null, JsonOps.INSTANCE, ExportItemComponents.registries);
        Map<String, Object> types = new HashMap<>();
        for (Object type : (Iterable<?>) ExportItemProperties.components)
            types.put(ExportItemProperties.name.invoke(ExportItemProperties.components, type).toString(), type);
        Class<?> component = Class.forName("kh");
        JsonArray cases = new JsonArray();
        for (JsonElement element : JsonParser.parseString(Files.readString(Path.of(args[0]))).getAsJsonArray()) {
            JsonObject request = element.getAsJsonObject();
            ExportInventoryTransfers.reset();
            Object menu = ExportInventoryTransfers.menu("minecraft:player");
            Object item = ExportInventoryTransfers.stack(request.get("item").getAsString(), request.get("count").getAsInt());
            for (var patch : request.getAsJsonObject("components").entrySet()) {
                Object type = Objects.requireNonNull(types.get(patch.getKey()));
                if (patch.getValue().isJsonNull()) ExportItemProperties.stack.getMethod("e", component).invoke(item, type);
                else {
                    Codec<?> codec = (Codec<?>) component.getMethod("b").invoke(type);
                    Object value = codec.parse(ops, patch.getValue()).getOrThrow();
                    ExportItemProperties.stack.getMethod("b", component, Object.class).invoke(item, type, value);
                }
            }
            int source = request.get("slot").getAsInt();
            List<?> slots = ExportInventoryTransfers.slots(menu);
            if (request.get("occupied").getAsBoolean())
                for (int slot : new int[]{5, 6, 7, 8, 45})
                    ExportInventoryTransfers.slotSet(slots.get(slot), ExportInventoryTransfers.stack("minecraft:dirt", 1));
            ExportInventoryTransfers.slotSet(slots.get(source), item);
            ExportInventoryTransfers.cursor(menu, ExportInventoryTransfers.empty);
            JsonObject row = new JsonObject();
            row.add("request", request);
            row.addProperty("input_hex", ExportItemProperties.encoded(item));
            row.addProperty("preferred_slot", ExportInventoryTransfers.playerClass.getMethod("f", ExportInventoryTransfers.stackClass)
                .invoke(ExportInventoryTransfers.player, item).toString());
            JsonArray accepted = new JsonArray();
            for (int slot : new int[]{5, 6, 7, 8, 45})
                accepted.add((Boolean) ExportInventoryTransfers.mayPlace.invoke(slots.get(slot), item));
            row.add("equipment_acceptance", accepted);
            row.add("before", snapshot(menu));
            ExportInventoryTransfers.quickMove(menu, source);
            row.add("after", snapshot(menu));
            row.addProperty("cursor_hex", ExportItemProperties.encoded(ExportInventoryTransfers.cursor(menu)));
            cases.add(row);
        }
        JsonObject tags = new JsonObject();
        java.util.stream.Stream<?> registries = (java.util.stream.Stream<?>) Class.forName("jf$a")
            .getMethod("c").invoke(ExportItemComponents.registries);
        for (Object lookup : registries.toList()) {
            Object key = Class.forName("jf$b").getMethod("g").invoke(lookup);
            String registry = Class.forName("amt").getMethod("a").invoke(key).toString();
            if (!Set.of("minecraft:item", "minecraft:entity_type").contains(registry)) continue;
            JsonObject group = new JsonObject();
            java.util.stream.Stream<?> declared = (java.util.stream.Stream<?>) Class.forName("jf").getMethod("e").invoke(lookup);
            for (Object tag : declared.toList()) {
                Object tagKey = Class.forName("jh$c").getMethod("h").invoke(tag);
                String name = Class.forName("bef").getMethod("b").invoke(tagKey).toString();
                JsonArray members = new JsonArray();
                for (Object holder : (Iterable<?>) tag)
                    members.add(Class.forName("jd").getMethod("g").invoke(holder).toString());
                group.add(name, members);
            }
            tags.add(registry, group);
        }
        JsonObject output = new JsonObject();
        output.add("cases", cases);
        output.add("builtin_tags", tags);
        Files.writeString(Path.of(args[1]), new GsonBuilder().serializeNulls().create().toJson(output) + "\n");
        Class.forName("bas").getMethod("close").invoke(ExportItemComponents.resourceManager);
        System.out.println("native equipment transfer cases=" + cases.size());
    }
}
