// Observe unchanged official Java 1.21.11 PLAYER_ACTION codec; no server is started.
import com.google.gson.*;
import io.netty.buffer.*;
import java.nio.file.*;
import java.util.*;

public final class ExportHandSwap {
    public static void main(String[] args) throws Exception {
        Class<?> packetType = Class.forName("aji"), actionType = Class.forName("aji$a");
        Class<?> positionType = Class.forName("is"), directionType = Class.forName("iz");
        Class<?> bufferType = Class.forName("wx"), streamType = Class.forName("aao");
        Object action = actionType.getField("g").get(null); // SWAP_ITEM_WITH_OFFHAND
        Object position = positionType.getField("c").get(null); // ZERO
        Object direction = directionType.getField("a").get(null); // DOWN
        Object codec = packetType.getField("a").get(null);
        JsonArray rows = new JsonArray();
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
                row.addProperty("sequence", sequence);
                row.addProperty("payload_hex", HexFormat.of().formatHex(bytes));
                rows.add(row);
            } finally { raw.release(); }
        }
        Files.writeString(Path.of(args[0]), new GsonBuilder().setPrettyPrinting().create().toJson(rows) + "\n");
    }
}
