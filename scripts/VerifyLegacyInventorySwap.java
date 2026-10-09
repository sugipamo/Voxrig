// Original test tooling for the unmodified official Java 1.16.1 server JAR.
// Decodes/re-encodes the frozen SWAP fixtures using the game's own packet codec.
// No network connection, game server or world is started.
import com.google.gson.*;
import io.netty.buffer.Unpooled;
import java.nio.file.*;
import java.util.*;

public final class VerifyLegacyInventorySwap {
    public static void main(String[] args) throws Exception {
        if (!u.a().getName().equals("1.16.1")) throw new IllegalStateException("version");
        uj.a();
        var records = new JsonParser().parse(Files.readString(Path.of(args[0]))).getAsJsonArray();
        var results = new JsonArray();
        for (var record : records) {
            var value = record.getAsJsonObject();
            byte[] bytes = HexFormat.of().parseHex(value.get("payload_hex").getAsString());
            var input = new mg(Unpooled.wrappedBuffer(bytes));
            var packet = new rj();
            packet.a(input);
            if (input.isReadable() || packet.b() != value.get("window_id").getAsInt()
                || packet.c() != value.get("source_slot").getAsInt()
                || packet.d() != value.get("hotbar").getAsInt()
                || packet.e() != value.get("action").getAsInt()
                || packet.g() != bgq.c || packet.f().a()
                || bke.a(packet.f().b()) != value.get("comparison_item_id").getAsInt()
                || packet.f().E() != value.get("comparison_count").getAsInt())
                throw new IllegalStateException("native decoded fields differ");
            int nativeId = mf.b.a(nj.a, packet);
            if (nativeId != 0x09) throw new IllegalStateException("native packet ID");
            var output = new mg(Unpooled.buffer());
            packet.b(output);
            byte[] encoded = new byte[output.readableBytes()];
            output.readBytes(encoded);
            if (!Arrays.equals(encoded, bytes)) throw new IllegalStateException("native roundtrip");
            var result = new JsonObject();
            for (String key : new String[]{"window_id", "source_slot", "hotbar", "action", "payload_hex", "comparison_item_id", "comparison_count"})
                result.add(key, value.get(key));
            result.addProperty("native_packet_id", nativeId);
            result.addProperty("native_roundtrip", true);
            results.add(result);
            input.release(); output.release();
        }
        Files.writeString(Path.of(args[1]), new Gson().toJson(results) + "\n");
        System.out.println(results.size() + " official native SWAP packet fixtures verified");
        v.h();
    }
}
