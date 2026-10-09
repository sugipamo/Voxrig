// Own calls to original menus with held cursors; no Minecraft method is replaced.
import com.google.gson.*;
import java.nio.file.*;
import java.util.*;
public final class ExportHeldCursorTransfers {
    static JsonArray snapshot(Object menu) throws Exception {
        JsonArray result = new JsonArray();
        for (Object slot : ExportInventoryTransfers.slots(menu))
            result.add(ExportItemProperties.encoded(ExportInventoryTransfers.slotGet(slot)));
        return result;
    }
    public static void main(String[] args) throws Exception {
        String version = args[0];
        ExportInventoryTransfers.init(version);
        ExportItemProperties.init(version);
        boolean legacy = ExportInventoryTransfers.legacy;
        JsonArray cases = new JsonArray();
        for (JsonElement element : new JsonParser().parse(Files.readString(Path.of(args[1]))).getAsJsonArray()) {
            JsonObject request = element.getAsJsonObject();
            ExportInventoryTransfers.reset();
            String name = request.get("menu").getAsString();
            Object menu = ExportInventoryTransfers.menu(name);
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
            String fixture = request.get("fixture").getAsString();
            List<?> slots = ExportInventoryTransfers.slots(menu);
            for (int i = 0; i < slots.size(); i++) {
                if (i == source || (name.equals("minecraft:player") && (i < 9 || i > 44))) continue;
                if (!fixture.equals("empty"))
                    ExportInventoryTransfers.slotSet(slots.get(i), ExportInventoryTransfers.stack(
                        fixture.equals("merge") ? "minecraft:stone" : "minecraft:dirt", fixture.equals("merge") ? 63 : 64));
            }
            ExportInventoryTransfers.slotSet(slots.get(source), ExportInventoryTransfers.stack("minecraft:stone", 7));
            Object held = ExportItemProperties.decode(HexFormat.of().parseHex(request.get("cursor_hex").getAsString()));
            ExportInventoryTransfers.cursor(menu, held);
            JsonObject row = new JsonObject();
            row.add("request", request);
            row.addProperty("actual_creative", actualCreative);
            row.add("before", snapshot(menu));
            row.addProperty("cursor_before_hex", ExportItemProperties.encoded(ExportInventoryTransfers.cursor(menu)));
            Object returned = ExportInventoryTransfers.quickMove(menu, source);
            row.add("after", snapshot(menu));
            row.addProperty("cursor_after_hex", ExportItemProperties.encoded(ExportInventoryTransfers.cursor(menu)));
            if (legacy) row.addProperty("legacy_return_hex", ExportItemProperties.encoded(returned));
            cases.add(row);
        }
        JsonObject output = new JsonObject();
        output.addProperty("version", version); output.add("cases", cases);
        Files.writeString(Path.of(args[2]), output.toString());
        if (legacy) Class.forName("v").getMethod("h").invoke(null);
        else Class.forName("bas").getMethod("close").invoke(ExportItemComponents.resourceManager);
    }
}
