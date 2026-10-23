// Own calls to original input SWAP branches; no game method is replaced.
// Empty recipes and constructor-default table access bound callback scope. These facts
// establish whole-stack destinations, not output recomputation or network receipts.
import com.google.gson.*;
import com.mojang.serialization.*;
import java.nio.file.*;
import java.util.*;
public final class ExportCraftingInputSwaps {
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
            int source = request.get("slot").getAsInt();
            int hotbar = request.get("hotbar").getAsInt(), destination = (table ? 37 : 36) + hotbar;
            List<?> slots = ExportInventoryTransfers.slots(menu);
            ExportInventoryTransfers.slotSet(slots.get(source), ExportCraftingReturns.stack(request.get("source")));
            ExportInventoryTransfers.slotSet(slots.get(destination), ExportCraftingReturns.stack(request.get("incoming")));
            ExportInventoryTransfers.cursor(menu, ExportInventoryTransfers.empty);
            JsonObject row = new JsonObject(); row.add("request", request);
            row.addProperty("actual_creative", actualCreative);
            row.add("before", snapshot(menu));
            row.addProperty("cursor_before_hex", ExportItemProperties.encoded(ExportInventoryTransfers.cursor(menu)));
            Object swap = Class.forName(legacy ? "bgq" : "dhu").getField("c").get(null);
            Object returned = ExportInventoryTransfers.menuClass.getMethod("a", int.class, int.class,
                Class.forName(legacy ? "bgq" : "dhu"), ExportInventoryTransfers.playerClass)
                .invoke(menu, source, hotbar, swap, ExportInventoryTransfers.player);
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
