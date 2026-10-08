// Original tooling for pinned unmodified official JARs. No server/world is started.
// Verifies close payloads with the native packet reader/writer; no game code is copied.
import com.google.gson.*;
import io.netty.buffer.ByteBuf;
import io.netty.buffer.Unpooled;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;

public final class VerifyContainerClose {
    public static void main(String[] args) throws Exception {
        boolean legacy = args[0].equals("1.16.1");
        if (!legacy && !args[0].equals("1.21.11")) throw new IllegalArgumentException("version");
        if (!legacy) Class.forName("w").getMethod("a").invoke(null);
        Object version = Class.forName(legacy ? "u" : "w").getMethod(legacy ? "a" : "b").invoke(null);
        String name = (String) Class.forName(legacy ? "com.mojang.bridge.game.GameVersion" : "aa")
            .getMethod(legacy ? "getName" : "c").invoke(version);
        if (!name.equals(args[0])) throw new IllegalStateException("native version mismatch");
        Class.forName(legacy ? "uj" : "amv").getMethod("a").invoke(null);
        Class<?> bufferClass = Class.forName(legacy ? "mg" : "wx");
        Class<?> packetClass = Class.forName(legacy ? "rk" : "ait");
        Constructor<?> bufferConstructor = bufferClass.getConstructor(ByteBuf.class);
        var inputCases = new JsonParser().parse(Files.readString(Path.of(args[1]))).getAsJsonArray();
        var results = new JsonArray();
        for (var entry : inputCases) {
            var value = entry.getAsJsonObject();
            if (!value.get("version").getAsString().equals(args[0])) continue;
            byte[] bytes = HexFormat.of().parseHex(value.get("payload_hex").getAsString());
            ByteBuf input = Unpooled.wrappedBuffer(bytes), output = Unpooled.buffer();
            Object inputBuffer = bufferConstructor.newInstance(input), outputBuffer = bufferConstructor.newInstance(output);
            try {
                Object packet;
                if (legacy) {
                    packet = packetClass.getConstructor().newInstance();
                    packetClass.getMethod("a", bufferClass).invoke(packet, inputBuffer);
                } else {
                    Constructor<?> reader = packetClass.getDeclaredConstructor(bufferClass);
                    reader.setAccessible(true);
                    packet = reader.newInstance(inputBuffer);
                }
                Field containerId = packetClass.getDeclaredField(legacy ? "a" : "b");
                containerId.setAccessible(true);
                if (input.isReadable() || containerId.getInt(packet) != value.get("window_id").getAsInt())
                    throw new IllegalStateException("native close decode mismatch");
                Method write = packetClass.getDeclaredMethod(legacy ? "b" : "a", bufferClass);
                write.setAccessible(true);
                write.invoke(packet, outputBuffer);
                byte[] encoded = new byte[output.readableBytes()]; output.readBytes(encoded);
                if (!Arrays.equals(encoded, bytes)) throw new IllegalStateException("native close roundtrip mismatch");
                var result = new JsonObject();
                for (String key : new String[]{"version", "window_id", "payload_hex"}) result.add(key,value.get(key));
                if (legacy) {
                    Object play = Class.forName("mf").getField("b").get(null);
                    Object serverbound = Class.forName("nj").getField("a").get(null);
                    int id = (Integer)Class.forName("mf").getMethod("a", Class.forName("nj"), Class.forName("ni")).invoke(play,serverbound,packet);
                    if (id != 0x0a) throw new IllegalStateException("native close packet ID");
                    result.addProperty("native_packet_id", id);
                } else {
                    String type = packetClass.getMethod("a").invoke(packet).toString();
                    if (!type.contains("container_close")) throw new IllegalStateException("native close packet type");
                    result.addProperty("native_packet_type", type);
                }
                result.addProperty("native_roundtrip", true); results.add(result);
            } finally { input.release(); output.release(); }
        }
        if (results.size() != (legacy ? 2 : 3)) throw new IllegalStateException("fixture coverage");
        Files.writeString(Path.of(args[2]), new Gson().toJson(results)+"\n");
        System.out.println(args[0]+": "+results.size()+" original native close codec cases verified");
        if (legacy) Class.forName("v").getMethod("h").invoke(null);
    }
}
