// Constructs and roundtrips packets through the unchanged official 1.21.11 codec.
import com.google.gson.*;
import io.netty.buffer.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;

public final class ExportEntityInteractAt {
    public static void main(String[] args) throws Exception {
        Class<?> buffer = Class.forName("wx"), packet = Class.forName("aiy");
        Class<?> hand = Class.forName("cdb"), vector = Class.forName("ftm");
        Constructor<?> action = Class.forName("aiy$e").getDeclaredConstructor(hand, vector);
        Constructor<?> make = packet.getDeclaredConstructor(int.class, boolean.class, Class.forName("aiy$a"));
        Constructor<?> read = packet.getDeclaredConstructor(buffer);
        Method write = packet.getDeclaredMethod("a", buffer);
        action.setAccessible(true); make.setAccessible(true); read.setAccessible(true); write.setAccessible(true);
        JsonArray rows = new JsonArray();
        for (int id : new int[]{-1, 0, 127, 128, 300, Integer.MAX_VALUE})
        for (float[] hit : new float[][]{{0, 0, 0}, {-0.0f, 1.5f, -0.25f},
                {Float.MAX_VALUE, -Float.MAX_VALUE, Float.MIN_VALUE}, {-1.25f, .1f, 2.5f}})
        for (int h = 0; h < 2; h++) for (boolean secondary : new boolean[]{false, true}) {
            Object location = vector.getConstructor(double.class, double.class, double.class)
                .newInstance((double)hit[0], (double)hit[1], (double)hit[2]);
            Object p = make.newInstance(id, secondary, action.newInstance(hand.getEnumConstants()[h], location));
            ByteBuf first = Unpooled.buffer(), second = Unpooled.buffer();
            try {
                write.invoke(p, buffer.getConstructor(ByteBuf.class).newInstance(first));
                byte[] bytes = new byte[first.readableBytes()]; first.getBytes(0, bytes);
                Object decoded = read.newInstance(buffer.getConstructor(ByteBuf.class).newInstance(first));
                if (first.isReadable()) throw new AssertionError("trailing bytes");
                write.invoke(decoded, buffer.getConstructor(ByteBuf.class).newInstance(second));
                byte[] roundtrip = new byte[second.readableBytes()]; second.readBytes(roundtrip);
                if (!Arrays.equals(bytes, roundtrip)) throw new AssertionError("roundtrip changed packet");
                JsonObject row = new JsonObject();
                row.addProperty("entity_id", id); row.addProperty("hand", h); row.addProperty("sneaking", secondary);
                JsonArray bits = new JsonArray(); for (float v : hit) bits.add(Integer.toUnsignedLong(Float.floatToRawIntBits(v)));
                row.add("hit_bits", bits); row.addProperty("payload_hex", HexFormat.of().formatHex(bytes)); rows.add(row);
            } finally { first.release(); second.release(); }
        }
        Files.writeString(Path.of(args[0]), new GsonBuilder().setPrettyPrinting().create().toJson(rows) + "\n");
    }
}
