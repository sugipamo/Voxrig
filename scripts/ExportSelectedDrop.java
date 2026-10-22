// Observe unchanged official Java 1.21.11 drop/resync codecs and no-op menu primitive.
// No server is started; setup helpers provide only an unspawned native context.
import com.google.gson.*;
import io.netty.buffer.*;
import java.nio.file.*;
import java.util.*;

public final class ExportSelectedDrop {
    public static void main(String[] args) throws Exception {
        Class<?> packetType = Class.forName("aji"), actionType = Class.forName("aji$a");
        Class<?> positionType = Class.forName("is"), directionType = Class.forName("iz");
        Class<?> bufferType = Class.forName("wx"), streamType = Class.forName("aao");
        Object position = positionType.getField("c").get(null); // ZERO
        Object direction = directionType.getField("a").get(null); // DOWN
        Object codec = packetType.getField("a").get(null);
        JsonArray rows = new JsonArray();
        for (String actionField : new String[]{"e", "d"}) {
            Object action = actionType.getField(actionField).get(null); // DROP_ITEM / DROP_ALL_ITEMS
            for (int sequence : new int[]{1, 127, 128, 16384, Integer.MAX_VALUE}) {
                Object packet = packetType.getConstructor(actionType, positionType, directionType, int.class)
                    .newInstance(action, position, direction, sequence);
                ByteBuf raw = Unpooled.buffer();
                try {
                    Object buffer = bufferType.getConstructor(ByteBuf.class).newInstance(raw);
                    streamType.getMethod("encode", Object.class, Object.class).invoke(codec, buffer, packet);
                    byte[] bytes = new byte[raw.readableBytes()];
                    raw.getBytes(raw.readerIndex(), bytes);
                    Object decoded = streamType.getMethod("decode", Object.class).invoke(codec, buffer);
                    if (raw.isReadable() || packetType.getMethod("f").invoke(decoded) != action
                        || !packetType.getMethod("b").invoke(decoded).equals(position)
                        || packetType.getMethod("e").invoke(decoded) != direction
                        || (int)packetType.getMethod("g").invoke(decoded) != sequence)
                        throw new IllegalStateException("original PLAYER_ACTION codec mismatch");
                    JsonObject row = new JsonObject();
                    row.addProperty("amount", actionField.equals("e") ? "one" : "all");
                    row.addProperty("sequence", sequence);
                    row.addProperty("payload_hex", HexFormat.of().formatHex(bytes));
                    rows.add(row);
                } finally { raw.release(); }
            }
        }
        ExportRegularClicks.init("1.21.11");
        Object equipment = Class.forName("cgo").getConstructor().newInstance();
        ExportRegularClicks.field(ExportRegularClicks.inventory, ExportRegularClicks.inventoryClass, "n", equipment);
        ExportRegularClicks.field(ExportRegularClicks.player, Class.forName("chl"), "cb", equipment);
        JsonObject refresh = new JsonObject();
        refresh.addProperty("payload_hex", HexFormat.of().formatHex(
            ExportRegularClicks.modernPacket(ExportRegularClicks.empty, 0, -1, -1, 0)));
        JsonArray checks = new JsonArray();
        for (int cursorCount : new int[]{0, 1, 7}) {
            Object menu = ExportRegularClicks.menu("minecraft:player");
            var slots = ExportRegularClicks.slots(menu);
            Object selected = ExportRegularClicks.stack("minecraft:dirt", 12);
            Object cursor = ExportRegularClicks.stack("minecraft:stone", cursorCount);
            ExportRegularClicks.slotSet(slots.get(36), selected);
            ExportRegularClicks.cursor(menu, cursor);
            var before = new ArrayList<Object>();
            var counts = new ArrayList<Integer>();
            for (Object slot : slots) {
                Object stack = ExportRegularClicks.slotGet(slot);
                before.add(stack);
                counts.add((int)ExportRegularClicks.count.invoke(stack));
            }
            if (!(boolean)ExportRegularClicks.menuClass.getMethod("a", int.class).invoke(menu, -1))
                throw new IllegalStateException("native rejects no-op slot");
            ExportRegularClicks.menuClass.getMethod("a", int.class, int.class,
                Class.forName("dhu"), ExportRegularClicks.playerClass)
                .invoke(menu, -1, 0, ExportRegularClicks.pickup, ExportRegularClicks.player);
            if (ExportRegularClicks.cursor(menu) != cursor ||
                (int)ExportRegularClicks.count.invoke(cursor) != cursorCount)
                throw new IllegalStateException("no-op changed native cursor");
            for (int i = 0; i < slots.size(); i++) {
                Object after = ExportRegularClicks.slotGet(slots.get(i));
                if (after != before.get(i) || (int)ExportRegularClicks.count.invoke(after) != counts.get(i))
                    throw new IllegalStateException("no-op changed native slot " + i);
            }
            JsonObject check = new JsonObject();
            check.addProperty("cursor_count", cursorCount);
            check.addProperty("selected_count", 12);
            check.addProperty("all_slots_and_cursor_unchanged", true);
            checks.add(check);
        }
        refresh.add("native_noop_cases", checks);
        Files.writeString(Path.of(args[1]), new GsonBuilder().setPrettyPrinting().create().toJson(refresh) + "\n");
        Files.writeString(Path.of(args[0]), new GsonBuilder().setPrettyPrinting().create().toJson(rows) + "\n");
    }
}
