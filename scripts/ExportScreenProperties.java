package voxrig.properties;
// Original ClientboundContainerSetDataPacket codec; constructed fixtures, not live captures.
import com.google.gson.*;
import io.netty.buffer.*;
import java.nio.file.*;
import java.util.*;

public final class ExportScreenProperties {
    static byte[] bytes(ByteBuf buffer) {
        byte[] bytes = new byte[buffer.readableBytes()];
        buffer.getBytes(buffer.readerIndex(), bytes);
        return bytes;
    }
    public static void main(String[] args) throws Exception {
        Class<?> bufferType = Class.forName("wx"), packetType = Class.forName("adw");
        Object codec = packetType.getField("a").get(null);
        Class<?> stream = Class.forName("aao");
        JsonArray rows = new JsonArray();
        for (int window : new int[]{1, 127, 300}) {
            for (int property : new int[]{Short.MIN_VALUE, -1, 0, 3, Short.MAX_VALUE}) {
                for (int value : new int[]{Short.MIN_VALUE, -1, 0, 200, Short.MAX_VALUE}) {
                    ByteBuf encoded = Unpooled.buffer(), reencoded = Unpooled.buffer();
                    try {
                        Object write = bufferType.getConstructor(ByteBuf.class).newInstance(encoded);
                        Object packet = packetType.getConstructor(int.class, int.class, int.class).newInstance(window, property, value);
                        stream.getMethod("encode", Object.class, Object.class).invoke(codec, write, packet);
                        byte[] original = bytes(encoded);
                        Object read = bufferType.getConstructor(ByteBuf.class).newInstance(encoded);
                        Object decoded = stream.getMethod("decode", Object.class).invoke(codec, read);
                        if (encoded.isReadable()) throw new AssertionError("unread fields");
                        Object output = bufferType.getConstructor(ByteBuf.class).newInstance(reencoded);
                        stream.getMethod("encode", Object.class, Object.class).invoke(codec, output, decoded);
                        if (!Arrays.equals(original, bytes(reencoded))) throw new AssertionError("roundtrip mismatch");
                        JsonObject row = new JsonObject();
                        row.addProperty("window", (int) packetType.getMethod("b").invoke(decoded));
                        row.addProperty("property", (int) packetType.getMethod("e").invoke(decoded));
                        row.addProperty("value", (int) packetType.getMethod("f").invoke(decoded));
                        row.addProperty("payload_hex", HexFormat.of().formatHex(original));
                        rows.add(row);
                    } finally { encoded.release(); reencoded.release(); }
                }
            }
        }
        JsonObject out = new JsonObject();
        out.addProperty("version", "1.21.11");
        out.addProperty("authority", "Constructed fields encoded, decoded and byte-for-byte re-encoded by unchanged official ClientboundContainerSetDataPacket.STREAM_CODEC; not live captures.");
        out.addProperty("server_jar_sha1", "64bb6d763bed0a9f1d632ec347938594144943ed");
        out.add("packets", rows);
        Files.writeString(Path.of(args[0]), new GsonBuilder().setPrettyPrinting().create().toJson(out) + "\n");
    }
}
