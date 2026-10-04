// Own reflection wrapper of untouched official 1.21.11 component/item codecs.
// No game method is replaced or redistributed. Primitive facts, not live receipt proof.
import com.google.gson.*;
import com.mojang.serialization.Codec;
import com.mojang.serialization.JsonOps;
import io.netty.buffer.ByteBuf;
import io.netty.buffer.Unpooled;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;

public final class ExportItemComponents {
    static Class<?> typeClass, codecClass, bufferClass;
    static Object registries;
    static Object buffer(ByteBuf bytes) throws Exception {
        return bufferClass.getConstructor(ByteBuf.class, Class.forName("jr")).newInstance(bytes, registries);
    }
    static byte[] roundtrip(Object codec, Object value) throws Exception {
        ByteBuf bytes = Unpooled.buffer();
        try {
            Object wire = buffer(bytes);
            codecClass.getMethod("encode", Object.class, Object.class).invoke(codec, wire, value);
            byte[] original = new byte[bytes.readableBytes()]; bytes.getBytes(0, original);
            Object decoded = codecClass.getMethod("decode", Object.class).invoke(codec, wire);
            if (bytes.isReadable()) throw new IllegalStateException("native codec left trailing data");
            bytes.clear();
            codecClass.getMethod("encode", Object.class, Object.class).invoke(codec, wire, decoded);
            byte[] encoded = new byte[bytes.readableBytes()]; bytes.getBytes(0, encoded);
            if (!Arrays.equals(original, encoded)) throw new IllegalStateException("native codec changed encoded value");
            return original;
        } finally { bytes.release(); }
    }
    static String hex(byte[] bytes) { return HexFormat.of().formatHex(bytes); }
    static JsonObject entry(String name, int id) {
        JsonObject value = new JsonObject(); value.addProperty("name", name); value.addProperty("native_id", id); return value;
    }
    public static void main(String[] args) throws Exception {
        if (args.length != 3) throw new IllegalArgumentException("samples, output, version");
        ExportInventoryTransfers.init(args[2]);
        if (ExportInventoryTransfers.legacy) throw new IllegalArgumentException("modern component oracle");
        typeClass = Class.forName("kh"); codecClass = Class.forName("aao"); bufferClass = Class.forName("xq");
        Object components = Class.forName("mi").getField("am").get(null);
        registries = Class.forName("jr").getMethod("a", ExportInventoryTransfers.registryClass)
                .invoke(null, Class.forName("mi").getField("aR").get(null));
        Map<String,Object> types = new TreeMap<>();
        JsonArray definitions = new JsonArray(), removed = new JsonArray(), samples = new JsonArray();
        JsonArray stacks = new JsonArray(), failures = new JsonArray();
        Map<String,Object> sampleValues = new HashMap<>();
        Object patchCodec = Class.forName("kg").getField("c").get(null);
        Object stackCodec = ExportInventoryTransfers.stackClass.getField("h").get(null);
        for (Object type : (Iterable<?>) components) {
            String name = ExportInventoryTransfers.nameOf.invoke(components, type).toString();
            int id = (int) ExportInventoryTransfers.idOf.invoke(components, type);
            types.put(name, type);
            Object stream = typeClass.getMethod("f").invoke(type);
            JsonObject definition = entry(name, id);
            definition.addProperty("stream_codec_class", stream.getClass().getName());
            definition.addProperty("persistent", typeClass.getMethod("b").invoke(type) != null);
            definitions.add(definition);
            Object builder = Class.forName("kg").getMethod("a").invoke(null);
            builder.getClass().getMethod("a", typeClass).invoke(builder, type);
            Object patch = builder.getClass().getMethod("a").invoke(builder);
            JsonObject removal = entry(name, id);
            removal.addProperty("patch_hex", hex(roundtrip(patchCodec, patch)));
            removed.add(removal);
        }
        JsonArray input = JsonParser.parseString(Files.readString(Path.of(args[0]))).getAsJsonArray();
        // Enumerate actual enum-codec values rather than guessing names/ordinals.
        for (Map.Entry<String,Object> entry : types.entrySet()) {
            Object stream = typeClass.getMethod("f").invoke(entry.getValue());
            if (!stream.getClass().getName().equals("aam$21")) continue;
            Field field = stream.getClass().getDeclaredField("a"); field.setAccessible(true);
            java.util.function.IntFunction<?> byId = (java.util.function.IntFunction<?>) field.get(stream);
            @SuppressWarnings("unchecked") Codec<Object> persistent =
                    (Codec<Object>) typeClass.getMethod("b").invoke(entry.getValue());
            for (int id : new int[]{0,1,7}) {
                JsonObject request = new JsonObject(); request.addProperty("name",entry.getKey());
                if (persistent == null) request.addProperty("native_enum_id",id);
                else request.add("value",persistent.encodeStart(JsonOps.INSTANCE,byId.apply(id)).getOrThrow());
                input.add(request);
            }
        }
        for (JsonElement element : input) {
            JsonObject request = element.getAsJsonObject();
            String name = request.get("name").getAsString();
            Object type = Objects.requireNonNull(types.get(name), "unknown original component " + name);
            int id = (int) ExportInventoryTransfers.idOf.invoke(components, type);
            try {
                Codec<?> persistent = (Codec<?>) typeClass.getMethod("b").invoke(type);
                Object stream = typeClass.getMethod("f").invoke(type);
                Object value;
                if (request.has("native_enum_id")) {
                    Field field = stream.getClass().getDeclaredField("a"); field.setAccessible(true);
                    value = ((java.util.function.IntFunction<?>)field.get(stream))
                            .apply(request.get("native_enum_id").getAsInt());
                } else value = persistent == null ? Class.forName("bhr").getField("a").get(null)
                        : persistent.parse(JsonOps.INSTANCE, request.get("value")).getOrThrow();
                JsonObject sample = entry(name, id);
                sample.add("requested_value", request.get("value"));
                sample.addProperty("value_class", value.getClass().getName());
                sample.addProperty("value_hex", hex(roundtrip(stream, value)));
                sampleValues.put(name,value);
                Object builder = Class.forName("kg").getMethod("a").invoke(null);
                builder.getClass().getMethod("a", typeClass, Object.class).invoke(builder, type, value);
                Object patch = builder.getClass().getMethod("a").invoke(builder);
                sample.addProperty("patch_hex", hex(roundtrip(patchCodec, patch)));
                samples.add(sample);
                Object stack = ExportInventoryTransfers.stack("minecraft:stone", 3);
                ExportInventoryTransfers.stackClass.getMethod("b", typeClass, Object.class).invoke(stack, type, value);
                JsonObject record = entry(name, id);
                record.add("requested_value", request.get("value"));
                record.addProperty("stack_hex", hex(roundtrip(stackCodec, stack)));
                record.addProperty("actual_patch_hex", hex(roundtrip(patchCodec,
                        ExportInventoryTransfers.stackClass.getMethod("d").invoke(stack))));
                JsonArray packets = new JsonArray();
                for (String label : new String[]{"storage_slot", "cursor", "raw_player_slot", "storage_full", "player_full"}) {
                    String packetName = switch(label) {
                        case "storage_slot" -> "adx"; case "cursor" -> "agm";
                        case "raw_player_slot" -> "agy"; default -> "adv";
                    };
                    Class<?> packetClass = Class.forName(packetName);
                    List<Object> content = new ArrayList<>(Collections.nCopies(label.equals("player_full")?46:63,ExportInventoryTransfers.empty));
                    content.set(label.equals("player_full")?9:27,stack);
                    content.set(content.size()-1,ExportInventoryTransfers.stack("minecraft:dirt",2));
                    Object packet = switch (packetName) {
                        case "adx" -> packetClass.getConstructor(int.class,int.class,int.class,ExportInventoryTransfers.stackClass)
                                .newInstance(3, 7, 27, stack);
                        case "agm" -> packetClass.getConstructor(ExportInventoryTransfers.stackClass).newInstance(stack);
                        case "agy" -> packetClass.getConstructor(int.class,ExportInventoryTransfers.stackClass).newInstance(9,stack);
                        default -> packetClass.getConstructor(int.class,int.class,List.class,ExportInventoryTransfers.stackClass)
                                .newInstance(label.equals("player_full")?0:3,7,content,stack);
                    };
                    JsonObject p = new JsonObject(); p.addProperty("class",packetName);
                    p.addProperty("label",label);
                    p.addProperty("payload_hex",hex(roundtrip(packetClass.getField("a").get(null),packet))); packets.add(p);
                }
                record.add("packets",packets); stacks.add(record);
            } catch (Exception failure) {
                JsonObject failed = entry(name,id); failed.add("requested_value",request.get("value"));
                failed.addProperty("exception",failure.toString());
                if(failure.getCause()!=null) failed.addProperty("cause",failure.getCause().toString());
                failures.add(failed);
            }
        }
        Object mixedBuilder=Class.forName("kg").getMethod("a").invoke(null);
        JsonArray mixedAdded=new JsonArray();
        for(String name:new String[]{"minecraft:custom_data","minecraft:custom_name","minecraft:damage",
                "minecraft:unbreakable","minecraft:custom_model_data","minecraft:block_state"}) {
            Object type=types.get(name), value=Objects.requireNonNull(sampleValues.get(name));
            mixedBuilder.getClass().getMethod("a",typeClass,Object.class).invoke(mixedBuilder,type,value);
            JsonObject added=entry(name,(int)ExportInventoryTransfers.idOf.invoke(components,type));
            added.addProperty("value_hex",hex(roundtrip(typeClass.getMethod("f").invoke(type),value)));
            mixedAdded.add(added);
        }
        mixedBuilder.getClass().getMethod("a",typeClass).invoke(mixedBuilder,types.get("minecraft:lore"));
        JsonObject mixed=new JsonObject();mixed.add("added",mixedAdded);
        mixed.addProperty("removed","minecraft:lore");
        mixed.addProperty("patch_hex",hex(roundtrip(patchCodec,mixedBuilder.getClass().getMethod("a").invoke(mixedBuilder))));
        JsonObject output = new JsonObject(); output.addProperty("version",args[2]);
        output.add("registry",definitions); output.add("removed",removed); output.add("samples",samples);
        output.add("stacks",stacks); output.add("failures",failures);
        output.add("mixed",mixed);
        Files.writeString(Path.of(args[1]),new GsonBuilder().setPrettyPrinting().create().toJson(output)+"\n");
    }
}
