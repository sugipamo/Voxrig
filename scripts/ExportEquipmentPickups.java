// Observe unchanged native player-menu PICKUP on exact equipment slots.
// Only original getters/setters and menu primitives; no game method bodies.
import com.google.gson.*;
import java.nio.file.*;

public final class ExportEquipmentPickups {
    public static void main(String[] args) throws Exception {
        ExportInventoryTransfers.init(args[0]);
        JsonArray cases = new JsonArray();
        String[] items = {"stone", "dirt", "shield", "iron_helmet", "iron_chestplate",
            "iron_leggings", "iron_boots", "elytra", "carved_pumpkin"};
        for (int index : new int[]{5, 6, 7, 8, 45})
            for (String source : items) for (String carried : items)
                for (int sc : new int[]{0, 1, 2}) for (int cc : new int[]{0, 1, 2})
                    for (int button = 0; button < 2; button++) {
                        ExportInventoryTransfers.reset();
                        Object menu = ExportInventoryTransfers.menu("minecraft:player");
                        Object slot = ExportInventoryTransfers.slots(menu).get(index);
                        Object original = ExportInventoryTransfers.stack("minecraft:" + source, sc);
                        Object cursor = ExportInventoryTransfers.stack("minecraft:" + carried, cc);
                        ExportInventoryTransfers.slotSet(slot, original);
                        ExportInventoryTransfers.cursor(menu, cursor);
                        JsonObject row = new JsonObject();
                        row.addProperty("slot", index); row.addProperty("button", button);
                        row.add("source_before", ExportInventoryTransfers.describe(ExportInventoryTransfers.slotGet(slot)));
                        row.add("cursor_before", ExportInventoryTransfers.describe(ExportInventoryTransfers.cursor(menu)));
                        row.addProperty("valid_counts", sc <= (int) ExportInventoryTransfers.stackClass
                            .getMethod(ExportInventoryTransfers.legacy ? "c" : "k").invoke(original)
                            && cc <= (int) ExportInventoryTransfers.stackClass
                            .getMethod(ExportInventoryTransfers.legacy ? "c" : "k").invoke(cursor));
                        Object returned = ExportInventoryTransfers.menuClass.getMethod("a", int.class, int.class,
                            Class.forName(ExportInventoryTransfers.legacy ? "bgq" : "dhu"),
                            ExportInventoryTransfers.playerClass).invoke(menu, index, button,
                            ExportInventoryTransfers.pickup, ExportInventoryTransfers.player);
                        row.add("source_after", ExportInventoryTransfers.describe(ExportInventoryTransfers.slotGet(slot)));
                        row.add("cursor_after", ExportInventoryTransfers.describe(ExportInventoryTransfers.cursor(menu)));
                        if (ExportInventoryTransfers.legacy)
                            row.add("legacy_returned", ExportInventoryTransfers.describe(returned));
                        cases.add(row);
                    }
        JsonObject result = new JsonObject(); result.addProperty("version", args[0]); result.add("cases", cases);
        Files.writeString(Path.of(args[1]), new GsonBuilder().serializeNulls().create().toJson(result) + "\n");
        System.out.println(args[0] + " exact equipment PICKUP cases=" + cases.size());
    }
}
