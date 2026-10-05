// Own observer of unchanged native ItemStack effective component getters.
import com.google.gson.*;
import java.nio.file.*;
import java.util.*;

public final class ExportEffectiveItemComponents {
    public static void main(String[] args) throws Exception {
        ExportItemProperties.init("1.21.11");
        JsonArray rows = new JsonArray(), values = new JsonArray();
        Map<String, Integer> seen = new HashMap<>();
        // Official mappings distinguish getComponents() from getPrototype().
        var fields = ExportItemProperties.stack.getMethod("a");
        var empty = ExportItemProperties.stack.getMethod("f");
        var count = ExportItemProperties.stack.getMethod("N");
        var typeGetter = Class.forName("kk").getMethod("a");
        var valueGetter = Class.forName("kk").getMethod("b");
        var codecGetter = Class.forName("kh").getMethod("f");
        for (JsonElement request : JsonParser.parseString(Files.readString(Path.of(args[0]))).getAsJsonArray()) {
            String wire = request.getAsString();
            Object stack = ExportItemProperties.decode(HexFormat.of().parseHex(wire));
            JsonObject row = new JsonObject();
            row.addProperty("input_hex", wire);
            row.addProperty("count", (Integer) count.invoke(stack));
            boolean isEmpty = (Boolean) empty.invoke(stack);
            row.addProperty("empty", isEmpty);
            JsonArray indices = new JsonArray();
            if (!isEmpty) for (Object typed : (Iterable<?>) fields.invoke(stack)) {
                Object type = typeGetter.invoke(typed), value = valueGetter.invoke(typed);
                int id = (Integer) ExportItemProperties.id.invoke(ExportItemProperties.components, type);
                String encoded = HexFormat.of().formatHex(ExportEnchantmentConstructors.encode(codecGetter.invoke(type), value));
                String key = id + ":" + encoded;
                Integer index = seen.get(key);
                if (index == null) {
                    index = values.size();
                    seen.put(key, index);
                    JsonObject field = new JsonObject();
                    field.addProperty("name", ExportItemProperties.name.invoke(ExportItemProperties.components, type).toString());
                    field.addProperty("native_id", id);
                    field.addProperty("value_hex", encoded);
                    values.add(field);
                }
                indices.add(index);
            }
            row.add("components", indices);
            rows.add(row);
        }
        JsonObject out = new JsonObject();
        out.add("cases", rows);
        out.add("values", values);
        out.addProperty("java_version", System.getProperty("java.version"));
        Files.writeString(Path.of(args[1]), ExportComponentNormalization.jsonUtf16(new GsonBuilder().serializeNulls().create().toJson(out)));
        Class.forName("bas").getMethod("close").invoke(ExportItemComponents.resourceManager);
        System.out.println("actual effective items=" + rows.size() + " values=" + values.size());
    }
}
