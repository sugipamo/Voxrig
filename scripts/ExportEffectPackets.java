// Own observer of unchanged official Java 1.21.11 transient packet codecs.
// Run with the original bundled server and libraries on the classpath.
import com.google.gson.*;
import io.netty.buffer.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;

public final class ExportEffectPackets {
    static Object registries;
    static final JsonArray packets = new JsonArray();
    static void variable(ByteBuf b, int value) {
        do {
            int next = value & 127;
            value >>>= 7;
            b.writeByte(next | (value != 0 ? 128 : 0));
        } while (value != 0);
    }
    static void string(ByteBuf b, String value) {
        byte[] bytes = value.getBytes(java.nio.charset.StandardCharsets.UTF_8);
        variable(b, bytes.length);
        b.writeBytes(bytes);
    }
    static long position() {
        return ((-17L & 0x3ffffff) << 38) | ((23L & 0x3ffffff) << 12) | (-60L & 0xfff);
    }
    static ByteBuf buffer() { return Unpooled.buffer(); }
    static byte[] bytes(ByteBuf b) {
        byte[] out = new byte[b.readableBytes()];
        b.getBytes(b.readerIndex(), out);
        return out;
    }
    static JsonObject observe(String name, int id, String type, ByteBuf raw) throws Exception {
        ByteBuf input = Unpooled.wrappedBuffer(bytes(raw)), output = buffer();
        try {
            Class<?> buf = Class.forName("xq");
            Constructor<?> ctor = buf.getConstructor(ByteBuf.class, Class.forName("jr"));
            Object read = ctor.newInstance(input, registries), write = ctor.newInstance(output, registries);
            Object codec = Class.forName(type).getField("a").get(null);
            Class<?> stream = Class.forName("aao");
            Object packet = stream.getMethod("decode", Object.class).invoke(codec, read);
            stream.getMethod("encode", Object.class, Object.class).invoke(codec, write, packet);
            if (input.isReadable()) throw new IllegalStateException("original codec left unread bytes: " + name);
            if (!Arrays.equals(bytes(raw), bytes(output))) throw new IllegalStateException("original codec changed fields: " + name);
            JsonObject row = new JsonObject();
            row.addProperty("name", name);
            row.addProperty("packet_id", id);
            row.addProperty("payload_hex", HexFormat.of().formatHex(bytes(output)));
            if (type.equals("aet")) {
                Field field = packet.getClass().getDeclaredField("l");
                field.setAccessible(true);
                row.addProperty("option_class", field.get(packet).getClass().getName());
            }
            packets.add(row);
            return row;
        } finally { input.release(); output.release(); raw.release(); }
    }
    static ByteBuf options(String name, int alternative) {
        ByteBuf b = buffer();
        switch (name) {
            case "block", "block_marker", "falling_dust", "dust_pillar", "block_crumble" -> variable(b, 123);
            case "entity_effect", "tinted_leaves", "flash" -> b.writeInt(0x81abcdef);
            case "dust" -> { b.writeInt(0x123456); b.writeFloat(1.25f); }
            case "dust_color_transition" -> { b.writeInt(0x123456); b.writeInt(0xabcdef); b.writeFloat(1.25f); }
            case "dragon_breath", "sculk_charge" -> b.writeFloat(0.75f);
            case "effect", "instant_effect" -> { b.writeInt(0x81abcdef); b.writeFloat(0.75f); }
            case "item" -> {
                variable(b, 2);
                variable(b, 1); // stone
                variable(b, alternative); variable(b, 0);
                if (alternative == 1) { variable(b, 3); variable(b, 7); } // damage=7
            }
            case "vibration" -> {
                variable(b, alternative);
                if (alternative == 0) b.writeLong(position());
                else { variable(b, 37); b.writeFloat(1.25f); }
                variable(b, 12);
            }
            case "trail" -> {
                b.writeDouble(-4.5); b.writeDouble(70.25); b.writeDouble(8.125);
                b.writeInt(0x81abcdef); variable(b, 12);
            }
            case "shriek" -> variable(b, 12);
        }
        return b;
    }
    static void particle(int id, String name, int alternative) throws Exception {
        ByteBuf b = buffer(), options = options(name, alternative);
        try {
            b.writeBoolean(true); b.writeBoolean(false);
            b.writeDouble(-12.5); b.writeDouble(64.25); b.writeDouble(8);
            b.writeFloat(0.25f); b.writeFloat(0.5f); b.writeFloat(-0.125f);
            b.writeFloat(0.75f); b.writeInt(alternative == 0 ? 0 : Integer.MAX_VALUE);
            variable(b, id); b.writeBytes(options);
            JsonObject row = observe("particle/" + name + "/" + alternative, 0x2e, "aet", b);
            row.addProperty("particle_id", id);
            row.addProperty("particle_name", "minecraft:" + name);
            row.addProperty("options_hex", HexFormat.of().formatHex(bytes(options)));
        } finally { options.release(); }
    }
    public static void main(String[] args) throws Exception {
        Class.forName("w").getMethod("a").invoke(null);
        Class.forName("amv").getMethod("a").invoke(null);
        registries = Class.forName("jr").getMethod("a", Class.forName("jq")).invoke(null, Class.forName("mi").getField("aR").get(null));
        Object registry = Class.forName("mi").getField("j").get(null);
        Class<?> reg = Class.forName("jq");
        for (Object p : (Iterable<?>)registry) {
            int id = (int)reg.getMethod("a", Object.class).invoke(registry, p);
            String name = reg.getMethod("b", Object.class).invoke(registry, p).toString().substring("minecraft:".length());
            particle(id, name, 0);
            if (name.equals("vibration") || name.equals("item")) particle(id, name, 1);
        }
        for (boolean entity : new boolean[] {false, true}) {
            for (int holder = 0; holder < 3; holder++) {
                ByteBuf b = buffer();
                variable(b, holder == 0 ? 1 : 0);
                if (holder != 0) { string(b, "voxrig:probe"); b.writeBoolean(holder == 2); if (holder == 2) b.writeFloat(24.5f); }
                variable(b, 7);
                if (entity) variable(b, 37);
                else { b.writeInt(-801); b.writeInt(-512); b.writeInt(1604); }
                b.writeFloat(0.75f); b.writeFloat(1.25f); b.writeLong(-29);
                observe("sound/" + entity + "/" + holder, entity ? 0x72 : 0x73, entity ? "ahg" : "ahh", b);
            }
        }
        for (int flags = 0; flags < 4; flags++) {
            ByteBuf b = buffer(); b.writeByte(flags);
            if ((flags & 1) != 0) variable(b, 7);
            if ((flags & 2) != 0) string(b, "voxrig:probe");
            observe("stop/" + flags, 0x75, "ahj", b);
        }
        ByteBuf world = buffer(); world.writeInt(2001); world.writeLong(position()); world.writeInt(-123); world.writeBoolean(true);
        observe("level", 0x2d, "aes", world);
        for (int progress : new int[] {0, 9, -1, 10, 127, -128}) {
            ByteBuf b = buffer(); variable(b, 37); b.writeLong(position()); b.writeByte(progress);
            observe("break/" + progress, 5, "adg", b);
        }
        ByteBuf pickup = buffer(); variable(pickup, 37); variable(pickup, 99); variable(pickup, 5);
        observe("pickup", 0x7a, "ahn", pickup);
        JsonObject out = new JsonObject();
        out.addProperty("version", "1.21.11");
        out.addProperty("authority", "Constructed payloads decoded and byte-for-byte re-encoded by unchanged official server codecs; original particle registry IDs, names and decoded option classes reflected.");
        out.addProperty("server_jar_sha1", "64bb6d763bed0a9f1d632ec347938594144943ed");
        out.addProperty("mappings_sha256", "7ffd98f77f403043748d56467aac287a58f8187a27d5d6b7ebcac70042b02b1c");
        out.add("packets", packets);
        Files.writeString(Path.of(args[0]), new GsonBuilder().setPrettyPrinting().create().toJson(out) + "\n");
    }
}
