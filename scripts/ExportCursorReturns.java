// Own wrapper of original native PICKUP, constructors and packet codecs.
// No game method is replaced. Primitive default-item return facts, not network/mode evidence.
import com.google.gson.*;
import java.nio.file.*;
import java.util.*;

public final class ExportCursorReturns {
    public static void main(String[] args) throws Exception {
        ExportInventoryTransfers.init(args[0]);
        boolean legacy = ExportInventoryTransfers.legacy;
        ExportInventoryTransfers.pickup = Class.forName(legacy ? "bgq" : "dhu").getField("a").get(null);
        var output = new JsonObject();
        output.addProperty("version", args[0]);
        var cases = new JsonArray();
        var packets = new JsonArray();
        var excluded = new JsonArray();
        for (String name : ExportInventoryTransfers.byName.keySet()) {
            Object sample = ExportInventoryTransfers.stack(name, 1);
            if ((boolean) ExportInventoryTransfers.isEmpty.invoke(sample)) continue;
            int maximum = (int) ExportInventoryTransfers.stackClass.getMethod(legacy ? "c" : "k").invoke(sample);
            boolean override = !legacy && (ExportInventoryTransfers.byName.get(name).getClass()
                    .getMethod("a", ExportInventoryTransfers.stackClass, ExportInventoryTransfers.slotClass,
                            Class.forName("dht"), ExportInventoryTransfers.playerClass).getDeclaringClass() != Class.forName("dlp")
                    || ExportInventoryTransfers.byName.get(name).getClass()
                    .getMethod("a", ExportInventoryTransfers.stackClass, ExportInventoryTransfers.stackClass,
                            ExportInventoryTransfers.slotClass, Class.forName("dht"),
                            ExportInventoryTransfers.playerClass, Class.forName("cic")).getDeclaringClass() != Class.forName("dlp"));
            for (int index : new int[] {27, 54}) {
                for (int held : Arrays.stream(new int[] {0, 1, maximum}).distinct().toArray()) {
                    for (int before : Arrays.stream(new int[] {0, maximum / 2, maximum - 1, maximum}).distinct().toArray()) {
                        // Nonempty item overrides are different operations (for
                        // example inserting into a bundle), not top-level return.
                        // Record this scope explicitly; never invent a no-op.
                        if (override && before > 0 && held > 0) {
                            var skip = new JsonObject();
                            skip.addProperty("item", name);
                            skip.addProperty("slot", index);
                            skip.addProperty("source_count", before);
                            skip.addProperty("cursor_count", held);
                            excluded.add(skip);
                            continue;
                        }
                        ExportInventoryTransfers.reset();
                        Object menu = ExportInventoryTransfers.menu("minecraft:generic_9x3");
                        Object slot = ExportInventoryTransfers.slots(menu).get(index);
                        ExportInventoryTransfers.slotSet(slot, ExportInventoryTransfers.stack(name, before));
                        ExportInventoryTransfers.cursor(menu, ExportInventoryTransfers.stack(name, held));
                        var record = new JsonObject();
                        record.addProperty("item", name);
                        record.addProperty("native_id", (int) ExportInventoryTransfers.idOf.invoke(ExportInventoryTransfers.items, ExportInventoryTransfers.byName.get(name)));
                        record.addProperty("maximum", maximum);
                        record.addProperty("slot", index);
                        record.add("source_before", ExportInventoryTransfers.describe(ExportInventoryTransfers.slotGet(slot)));
                        record.add("cursor_before", ExportInventoryTransfers.describe(ExportInventoryTransfers.cursor(menu)));
                        Object returned = ExportInventoryTransfers.menuClass.getMethod("a", int.class, int.class,
                                Class.forName(legacy ? "bgq" : "dhu"), ExportInventoryTransfers.playerClass)
                                .invoke(menu, index, 0, ExportInventoryTransfers.pickup, ExportInventoryTransfers.player);
                        record.add("source_after", ExportInventoryTransfers.describe(ExportInventoryTransfers.slotGet(slot)));
                        record.add("cursor_after", ExportInventoryTransfers.describe(ExportInventoryTransfers.cursor(menu)));
                        if (legacy) record.add("legacy_returned", ExportInventoryTransfers.describe(returned));
                        if (!legacy) {
                            // Reject a hidden non-default component delta instead
                            // of silently describing it as a plain default stack.
                            ExportInventoryTransfers.modernPacket(ExportInventoryTransfers.slotGet(slot), 3, 7, index, 0);
                            ExportInventoryTransfers.modernPacket(ExportInventoryTransfers.cursor(menu), 3, 7, index, 0);
                        }
                        cases.add(record);
                    }
                }
                // Original codecs use the exact carried constructor data;
                // a nonempty predecessor against native Empty return forces resync.
                Object cursor = ExportInventoryTransfers.stack(name, 1);
                var packet = new JsonObject();
                packet.addProperty("item", name);
                packet.addProperty("slot", index);
                packet.add("cursor", ExportInventoryTransfers.describe(cursor));
                byte[] bytes = legacy
                        ? ExportInventoryTransfers.legacyPacket(cursor, 3, index, index, 0)
                        : ExportInventoryTransfers.modernPacket(cursor, 3, 7, index, 0);
                packet.addProperty("payload_hex", HexFormat.of().formatHex(bytes));
                packets.add(packet);
            }
        }
        output.add("cases", cases);
        output.add("packets", packets);
        output.add("excluded_nonempty_override_cases", excluded);
        Files.writeString(Path.of(args[1]), new GsonBuilder().setPrettyPrinting().create().toJson(output) + "\n");
    }
}
