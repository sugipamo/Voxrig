// Own wrapper of original NbtIo, Tag equality and CompoundTag persistent HashOps.
// Native bytecode/methods are neither changed nor copied into the output.
import com.google.gson.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;

public final class ExportNbtSemantics {
    static boolean legacy;
    static Class<?> io, tagClass, accountClass;
    static Method reader, writer;
    static Object account() throws Exception {
        return legacy ? accountClass.getConstructor(long.class).newInstance(16L * 1024 * 1024)
            : accountClass.getMethod("a", long.class).invoke(null, 16L * 1024 * 1024);
    }
    static Object read(byte[] bytes) throws Exception {
        DataInputStream in = new DataInputStream(new ByteArrayInputStream(bytes));
        Object value = legacy ? reader.invoke(null, in, 0, account()) : reader.invoke(null, in, account());
        if (in.available() != 0) throw new IllegalArgumentException("trailing NBT input");
        return value;
    }
    static byte[] write(Object value) throws Exception {
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        writer.invoke(null, value, new DataOutputStream(out));
        return out.toByteArray();
    }
    static int persistentHash(Object value) throws Exception {
        Object codec = Class.forName("uz").getField("a").get(null);
        Object ops = Class.forName("bfw").getField("c").get(null);
        Object result = Class.forName("com.mojang.serialization.Encoder")
            .getMethod("encodeStart", Class.forName("com.mojang.serialization.DynamicOps"), Object.class)
            .invoke(codec, ops, value);
        Object hash = Class.forName("com.mojang.serialization.DataResult").getMethod("getOrThrow").invoke(result);
        return (int) Class.forName("com.google.common.hash.HashCode").getMethod("asInt").invoke(hash);
    }
    static Object payload(Object value, Class<?> type) throws Exception {
        for (Field field : value.getClass().getDeclaredFields()) {
            if (!Modifier.isStatic(field.getModifiers()) && type.isAssignableFrom(field.getType())) {
                field.setAccessible(true); return field.get(value);
            }
        }
        throw new IllegalArgumentException("no native payload field: " + value.getClass());
    }
    static JsonArray units(String value) {
        JsonArray out = new JsonArray(); for (char c : value.toCharArray()) out.add((int)c); return out;
    }
    static JsonObject describe(Object value) throws Exception {
        int kind = ((Number) tagClass.getMethod(legacy ? "a" : "b").invoke(value)).intValue();
        JsonObject out = new JsonObject(); out.addProperty("kind", kind);
        switch (kind) {
            case 1: out.addProperty("value", (Byte)payload(value, byte.class)); break;
            case 2: out.addProperty("value", (Short)payload(value, short.class)); break;
            case 3: out.addProperty("value", (Integer)payload(value, int.class)); break;
            case 4: out.addProperty("value", (Long)payload(value, long.class)); break;
            case 5: out.addProperty("bits", Integer.toUnsignedLong(Float.floatToRawIntBits((Float)payload(value, float.class)))); break;
            case 6: out.addProperty("bits", Long.toUnsignedString(Double.doubleToRawLongBits((Double)payload(value, double.class)))); break;
            case 8: out.add("units", units((String)payload(value, String.class))); break;
            case 7: case 11: case 12: {
                Object array = payload(value, kind == 7 ? byte[].class : kind == 11 ? int[].class : long[].class);
                JsonArray entries = new JsonArray(); for (int i=0;i<Array.getLength(array);i++) entries.add((Number)Array.get(array,i));
                out.add("values", entries); break;
            }
            case 9: {
                JsonArray entries = new JsonArray(); for(Object child : (Iterable<?>)value) entries.add(describe(child));
                out.add("values", entries); break;
            }
            case 10: {
                @SuppressWarnings("unchecked") Map<String,Object> map = (Map<String,Object>)payload(value, Map.class);
                JsonArray entries = new JsonArray();
                for(Map.Entry<String,Object> entry : new TreeMap<>(map).entrySet()) {
                    JsonObject e = new JsonObject(); e.add("key_units",units(entry.getKey())); e.add("value",describe(entry.getValue())); entries.add(e);
                }
                out.add("entries", entries); break;
            }
            default: throw new IllegalArgumentException("unexpected native tag kind " + kind);
        }
        return out;
    }
    static JsonObject customDataPrototypes() throws Exception {
        Class.forName("w").getMethod("a").invoke(null);
        Object version = Class.forName("w").getMethod("b").invoke(null);
        if (!Class.forName("aa").getMethod("c").invoke(version).equals("1.21.11"))
            throw new IllegalStateException("original game version mismatch");
        Class.forName("amv").getMethod("a").invoke(null);
        Object items = Class.forName("mi").getField("h").get(null);
        Object customData = Class.forName("ki").getField("b").get(null);
        Constructor<?> constructor = Class.forName("dlt").getConstructor(Class.forName("dwn"), int.class);
        Method get = Class.forName("kd").getMethod("a", Class.forName("kh"));
        Method name = Class.forName("jq").getMethod("b", Object.class);
        JsonArray present = new JsonArray(); int count = 0;
        for (Object item : (Iterable<?>) items) {
            count++;
            if (get.invoke(constructor.newInstance(item, 1), customData) != null)
                present.add(name.invoke(items, item).toString());
        }
        JsonObject result = new JsonObject(); result.addProperty("items", count); result.add("items_with_custom_data", present);
        return result;
    }
    public static void main(String[] args) throws Exception {
        String version = args[0]; legacy = version.equals("1.16.1");
        if (!legacy && !version.equals("1.21.11")) throw new IllegalArgumentException("pinned versions only");
        io = Class.forName(legacy ? "lo" : "vm");
        tagClass = Class.forName(legacy ? "lu" : "vz");
        accountClass = Class.forName(legacy ? "ln" : "vi");
        reader = legacy ? io.getDeclaredMethod("a", DataInput.class, int.class, accountClass)
            : io.getMethod("b", DataInput.class, accountClass);
        reader.setAccessible(true);
        writer = io.getDeclaredMethod("a", tagClass, DataOutput.class); writer.setAccessible(true);
        JsonArray requests = new JsonParser().parse(Files.readString(Path.of(args[1]))).getAsJsonArray();
        JsonArray values = new JsonArray(), failures = new JsonArray(), pairs = new JsonArray();
        List<Object> nativeValues = new ArrayList<>();
        List<String> names = new ArrayList<>();
        for (JsonElement element : requests) {
            JsonObject request = element.getAsJsonObject();
            String name = request.get("name").getAsString();
            String hex = request.get(legacy ? "named_hex" : "unnamed_hex").getAsString();
            JsonObject row = new JsonObject(); row.addProperty("name", name); row.addProperty("input_hex", hex);
            try {
                Object value = read(HexFormat.of().parseHex(hex));
                row.addProperty("native_class", value.getClass().getName());
                row.addProperty("canonical_hex", HexFormat.of().formatHex(write(value)));
                row.addProperty("java_hash", value.hashCode());
                row.add("decoded", describe(value));
                if (!legacy) row.addProperty("persistent_crc32c", persistentHash(value));
                Object decoded = read(write(value));
                row.addProperty("canonical_decode_equal", value.equals(decoded));
                values.add(row); nativeValues.add(value); names.add(name);
            } catch (Throwable failure) {
                Throwable cause = failure;
                while (cause instanceof InvocationTargetException && cause.getCause() != null) cause = cause.getCause();
                row.addProperty("exception", cause.getClass().getName());
                row.addProperty("message", String.valueOf(cause.getMessage())); failures.add(row);
            }
        }
        for (int i = 0; i < nativeValues.size(); i++) {
            for (int j = i; j < nativeValues.size(); j++) {
                JsonObject row = new JsonObject(); row.addProperty("left", names.get(i)); row.addProperty("right", names.get(j));
                row.addProperty("equal", nativeValues.get(i).equals(nativeValues.get(j))); pairs.add(row);
            }
        }
        JsonObject output = new JsonObject(); output.addProperty("version", version);
        output.add("values", values); output.add("pairs", pairs); output.add("failures", failures);
        if (!legacy) output.add("default_custom_data_prototypes", customDataPrototypes());
        Files.writeString(Path.of(args[2]), new GsonBuilder().setPrettyPrinting().create().toJson(output) + "\n");
    }
}
