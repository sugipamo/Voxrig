package voxrig.explosion;
// Constructed packets round-tripped through the unchanged official 1.21.11 codec.
// Run with the original server and libraries on the classpath in a disposable directory.
import com.google.gson.*;
import io.netty.buffer.*;
import java.nio.file.*;
import java.lang.reflect.*;
import java.util.*;

public final class ExportExplosionPackets {
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
            packets.add(row);
            return row;
        } finally { input.release(); output.release(); raw.release(); }
    }

    public static void main(String[] args) throws Exception {
        Class.forName("w").getMethod("a").invoke(null);
        Class.forName("amv").getMethod("a").invoke(null);
        registries = Class.forName("jr").getMethod("a", Class.forName("jq")).invoke(null, Class.forName("mi").getField("aR").get(null));
        for (int impulse = 0; impulse < 3; impulse++) {
            for (int presentation = 0; presentation < 3; presentation++) {
                ByteBuf b = buffer();
                b.writeDouble(-12.5); b.writeDouble(64.25); b.writeDouble(8.125);
                b.writeFloat(4.5f); b.writeInt(-123);
                b.writeBoolean(impulse != 0);
                if (impulse != 0) {
                    b.writeDouble(impulse == 1 ? 0 : -0.25);
                    b.writeDouble(impulse == 1 ? 0 : 0.5);
                    b.writeDouble(impulse == 1 ? 0 : 1.25);
                }
                variable(b, presentation == 2 ? 14 : 22);
                if (presentation == 2) { b.writeInt(0x123456); b.writeFloat(1.25f); }
                variable(b, presentation == 0 ? 1 : 0);
                if (presentation != 0) {
                    string(b, "voxrig:explosion");
                    b.writeBoolean(presentation == 2);
                    if (presentation == 2) b.writeFloat(24.5f);
                }
                variable(b, presentation == 0 ? 0 : 2);
                if (presentation != 0) {
                    variable(b, 1); variable(b, 123); // block particle
                    b.writeFloat(0.75f); b.writeFloat(1.25f); variable(b, 0);
                    variable(b, 23); // explosion particle
                    b.writeFloat(-0.5f); b.writeFloat(0); variable(b, 3);
                }
                observe("explosion/" + impulse + "/" + presentation, 0x24, "aek", b);
            }
        }
        JsonObject out = new JsonObject();
        out.addProperty("version", "1.21.11");
        out.addProperty("authority", "Constructed payloads decoded and byte-for-byte re-encoded by unchanged official ClientboundExplodePacket.STREAM_CODEC, not live captures.");
        out.addProperty("server_jar_sha1", "64bb6d763bed0a9f1d632ec347938594144943ed");
        out.add("packets", packets);
        Files.writeString(Path.of(args[0]), new GsonBuilder().setPrettyPrinting().create().toJson(out) + "\n");
    }
}
