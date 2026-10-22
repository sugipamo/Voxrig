// Own calls to original input QUICK_MOVE branches; no game method is replaced.
// Empty recipes and constructor-default table access bound callback scope. These facts
// establish input routing/merging, not output recomputation or network receipts.
import com.google.gson.*;
import com.mojang.serialization.*;
import java.nio.file.*;
import java.util.*;
public final class ExportCraftingInputTransfers {
    static JsonArray snapshot(Object menu) throws Exception {
        JsonArray result = new JsonArray();
        for (Object slot : ExportInventoryTransfers.slots(menu))
            result.add(ExportItemProperties.encoded(ExportInventoryTransfers.slotGet(slot)));
        return result;
    }
    static Object stack(String item, int count, String name) throws Exception {
        JsonObject row = new JsonObject();
        row.addProperty("item", item); row.addProperty("count", count);
        if (name != null) row.addProperty("name", name);
        return ExportCraftingReturns.stack(row);
    }
    @SuppressWarnings("unchecked") public static void main(String[] args) throws Exception {
        String version = args[0];
        ExportInventoryTransfers.init(version);
        ExportItemProperties.init(version);
        boolean legacy = ExportInventoryTransfers.legacy;
        ExportCraftingReturns.old = legacy;
        if (!legacy) {
            for (Object type : (Iterable<?>) ExportItemProperties.components)
                ExportCraftingReturns.types.put(ExportItemProperties.name.invoke(ExportItemProperties.components, type).toString(), type);
            ExportCraftingReturns.ops = (DynamicOps<JsonElement>) Class.forName("ams")
                .getMethod("a", DynamicOps.class, Class.forName("jf$a"))
                .invoke(null, JsonOps.INSTANCE, ExportItemComponents.registries);
        }
        // Original callbacks run against an empty native recipe manager. The
        // modern native connection queues output packets without opening a socket.
        // Legacy uses its original client-side callback guard; neither context
        // claims to exercise real recipe output changes.
        Object world = legacy ? Class.forName("aom").getField("l").get(ExportInventoryTransfers.player)
            : ExportInventoryTransfers.playerClass.getMethod("ao").invoke(ExportInventoryTransfers.player);
        if (legacy) ExportInventoryTransfers.field(world, Class.forName("bqb"), "v", true);
        else {
            Object server = Class.forName("axf").getMethod("s").invoke(world);
            Object resources = ExportInventoryTransfers.allocate(Class.forName("ane"));
            Object recipes = Class.forName("dqz").getConstructor(Class.forName("jf$a")).newInstance(ExportItemComponents.registries);
            ExportInventoryTransfers.field(resources, Class.forName("ane"), "e", recipes);
            var constructor = Class.forName("net.minecraft.server.MinecraftServer$a").getDeclaredConstructors()[0];
            constructor.setAccessible(true);
            Object bundle = constructor.newInstance(null, resources);
            ExportInventoryTransfers.field(server, Class.forName("net.minecraft.server.MinecraftServer"), "aD", bundle);
            Object connection = Class.forName("wu").getConstructor(Class.forName("aaz"))
                .newInstance(Class.forName("aaz").getEnumConstants()[0]);
            Object listener = ExportInventoryTransfers.allocate(Class.forName("ayi"));
            ExportInventoryTransfers.field(listener, Class.forName("ayf"), "e", connection);
            ExportInventoryTransfers.field(ExportInventoryTransfers.player, Class.forName("axg"), "g", listener);
        }
        JsonArray cases = new JsonArray();
        for (JsonElement element : new JsonParser().parse(Files.readString(Path.of(args[1]))).getAsJsonArray()) {
            JsonObject request = element.getAsJsonObject();
            ExportInventoryTransfers.reset();
            boolean table = request.get("menu").getAsString().equals("minecraft:crafting");
            Object menu = table ? Class.forName(legacy ? "bgv" : "dic")
                .getConstructor(int.class, ExportInventoryTransfers.inventoryClass).newInstance(3, ExportInventoryTransfers.inventory)
                : ExportInventoryTransfers.menu("minecraft:player");
            boolean creative = request.get("mode").getAsString().equals("creative");
            Object gameMode = ExportInventoryTransfers.player.getClass()
                .getField(legacy ? "d" : "h").get(ExportInventoryTransfers.player);
            Object gameType = Class.forName(legacy ? "bpy" : "dwl")
                .getField(legacy ? (creative ? "c" : "b") : (creative ? "b" : "a")).get(null);
            ExportInventoryTransfers.field(gameMode, Class.forName(legacy ? "zf" : "axh"), legacy ? "d" : "e", gameType);
            boolean actualCreative = (Boolean) ExportInventoryTransfers.playerClass
                .getMethod(legacy ? "b_" : "ha").invoke(ExportInventoryTransfers.player);
            if (actualCreative != creative) throw new IllegalStateException("actual mode differs");
            int source = request.get("slot").getAsInt(), start = table ? 10 : 9, end = start + 36;
            String fixture = request.get("fixture").getAsString(), item = request.get("item").getAsString();
            String data = request.get("data").getAsBoolean() ? "input" : null;
            List<?> slots = ExportInventoryTransfers.slots(menu);
            int max = (Integer) ExportItemProperties.stack.getMethod(legacy ? "c" : "k").invoke(stack(item, 1, data));
            if (!fixture.equals("empty") && !fixture.equals("merge-late"))
                for (int i = start; i < end; i++)
                    ExportInventoryTransfers.slotSet(slots.get(i), stack("minecraft:dirt", 64, null));
            if (fixture.equals("partial") || fixture.equals("merge-late") || fixture.equals("split"))
                ExportInventoryTransfers.slotSet(slots.get(end - 1), stack(item, Math.max(1, max - 1), data));
            if (fixture.equals("split"))
                ExportInventoryTransfers.slotSet(slots.get(start), stack(item, Math.max(1, max - 2), data));
            if (fixture.equals("unequal"))
                ExportInventoryTransfers.slotSet(slots.get(start), stack(item, 1, "other"));
            ExportInventoryTransfers.slotSet(slots.get(source), stack(item, Math.min(7, max), data));
            ExportInventoryTransfers.cursor(menu, request.get("held").getAsBoolean()
                ? stack("minecraft:diamond_helmet", 1, "cursor") : ExportInventoryTransfers.empty);
            JsonObject row = new JsonObject(); row.add("request", request);
            row.addProperty("actual_creative", actualCreative);
            row.add("before", snapshot(menu));
            row.addProperty("cursor_before_hex", ExportItemProperties.encoded(ExportInventoryTransfers.cursor(menu)));
            Object returned = ExportInventoryTransfers.quickMove(menu, source);
            row.add("after", snapshot(menu));
            row.addProperty("cursor_after_hex", ExportItemProperties.encoded(ExportInventoryTransfers.cursor(menu)));
            if (legacy) row.addProperty("legacy_return_hex", ExportItemProperties.encoded(returned));
            cases.add(row);
        }
        JsonObject output = new JsonObject(); output.addProperty("version", version); output.add("cases", cases);
        Files.writeString(Path.of(args[2]), output.toString());
        if (legacy) Class.forName("v").getMethod("h").invoke(null);
        else Class.forName("bas").getMethod("close").invoke(ExportItemComponents.resourceManager);
    }
}
