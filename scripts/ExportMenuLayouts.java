// Original tooling: enumerate the unmodified official native registry and
// construct menu factories without a server/world. No game code is distributed.
import com.google.gson.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;

public final class ExportMenuLayouts {
    public static void main(String[] args) throws Exception {
        boolean legacy = args[0].equals("1.16.1");
        if (!legacy && !args[0].equals("1.21.11")) throw new IllegalArgumentException("version");
        if (!legacy) Class.forName("w").getMethod("a").invoke(null);
        Object version = Class.forName(legacy ? "u" : "w").getMethod(legacy ? "a" : "b").invoke(null);
        String versionName = (String) Class.forName(legacy ? "com.mojang.bridge.game.GameVersion" : "aa")
            .getMethod(legacy ? "getName" : "c").invoke(version);
        if (!versionName.equals(args[0])) throw new IllegalStateException("native version mismatch");
        Class.forName(legacy ? "uj" : "amv").getMethod("a").invoke(null);
        Class<?> registryClass = Class.forName(legacy ? "gl" : "jq");
        Object registry = Class.forName(legacy ? "gl" : "mi")
            .getField(legacy ? "aM" : "q").get(null);
        Class<?> inventoryClass = Class.forName(legacy ? "beb" : "ddl");
        Constructor<?> constructor = inventoryClass.getConstructors()[0];
        Object inventory = constructor.newInstance(new Object[constructor.getParameterCount()]);
        Field slotsField = Class.forName(legacy ? "bgi" : "dhi").getField(legacy ? "a" : "k");
        Class<?> slotClass = Class.forName(legacy ? "bhw" : "dji");
        Field containerField = slotClass.getField("c");
        Field rawIndexField = slotClass.getDeclaredField("a");
        rawIndexField.setAccessible(true);
        var records = new JsonArray();
        int layouts = 0;
        for (Object type : (Iterable<?>) registry) {
            var value = new JsonObject();
            value.addProperty("native_id", (Integer) registryClass.getMethod("a", Object.class).invoke(registry, type));
            value.addProperty("name", registryClass.getMethod("b", Object.class).invoke(registry, type).toString());
            try {
                String name = value.get("name").getAsString();
                String nativeClass;
                int size;
                int rows = name.startsWith("minecraft:generic_9x") ? Integer.parseInt(name.substring(name.length() - 1)) : 0;
                if (rows != 0) { nativeClass = legacy ? "bgp" : "dhs"; size = rows * 9; }
                else if (name.equals("minecraft:generic_3x3")) { nativeClass = legacy ? "bgx" : "die"; size = 9; }
                else if (name.equals("minecraft:hopper")) { nativeClass = legacy ? "bhd" : "dik"; size = 5; }
                else if (name.equals("minecraft:shulker_box")) { nativeClass = legacy ? "bht" : "djf"; size = 27; }
                else { value.addProperty("unavailable_reason", "constructor not yet inspected by this oracle"); records.add(value); continue; }
                Object container = Class.forName(legacy ? "anm" : "cdk").getConstructor(int.class).newInstance(size);
                Constructor<?> factory = Arrays.stream(Class.forName(nativeClass).getDeclaredConstructors())
                    .filter(c -> c.getParameterCount() == (rows != 0 ? 5 : 3)).findFirst().orElseThrow();
                factory.setAccessible(true);
                Object menu = rows != 0 ? factory.newInstance(type, 1, inventory, container, rows)
                    : factory.newInstance(1, inventory, container);
                List<?> slots = (List<?>) slotsField.get(menu);
                var player = new JsonArray();
                for (int i = 0; i < slots.size(); i++) {
                    if (containerField.get(slots.get(i)) == inventory) {
                        var mapping = new JsonObject();
                        mapping.addProperty("screen_slot", i);
                        mapping.addProperty("raw_player_slot", rawIndexField.getInt(slots.get(i)));
                        player.add(mapping);
                    }
                }
                value.addProperty("total_slots", slots.size());
                value.add("player_slots", player);
                layouts++;
            } catch (InvocationTargetException failure) {
                // No guessed layout for factories needing an actual player/world.
                value.addProperty("unavailable_reason", failure.getCause().toString());
            }
            records.add(value);
        }
        if (layouts != 9 || records.size() != (legacy ? 24 : 25))
            throw new IllegalStateException("native registry/layout coverage differs");
        Files.writeString(Path.of(args[1]), new GsonBuilder().setPrettyPrinting().create().toJson(records) + "\n");
        System.out.println(args[0] + ": " + records.size() + " native registry entries and 9 storage layouts inspected");
        if (legacy) Class.forName("v").getMethod("h").invoke(null);
    }
}
