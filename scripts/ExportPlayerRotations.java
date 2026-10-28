package voxrig.rotation;
// ClientboundPlayerRotationPacket (afq) encoded by the unchanged official 1.21.11
// codec, and resolved by the unchanged PositionMoveRotation.calculateAbsolute (chy)
// with Relative.rotation (chz), exactly as ClientPacketListener.handleRotatePlayer does.
// Run with the original server and libraries on the classpath in a disposable directory:
//   java -cp 'versions/1.21.11/server-1.21.11.jar:libraries/*:<classes>' voxrig.rotation.ExportPlayerRotations out.json
import com.google.gson.*;
import io.netty.buffer.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;

public final class ExportPlayerRotations {
    static JsonArray pair(float a, float b) { JsonArray o = new JsonArray(); o.add(a); o.add(b); return o; }

    public static void main(String[] args) throws Exception {
        Class.forName("w").getMethod("a").invoke(null); // SharedConstants.tryDetectVersion
        Class.forName("amv").getMethod("a").invoke(null); // Bootstrap.bootStrap
        Object registries = Class.forName("jr").getMethod("a", Class.forName("jq"))
            .invoke(null, Class.forName("mi").getField("aR").get(null));
        Class<?> packetType = Class.forName("afq"), pmr = Class.forName("chy"), vec = Class.forName("ftm");
        Class<?> relative = Class.forName("chz"), stream = Class.forName("aao"), buf = Class.forName("xq");
        Constructor<?> packetCtor = packetType.getConstructor(float.class, boolean.class, float.class, boolean.class);
        Constructor<?> pmrCtor = pmr.getConstructor(vec, vec, float.class, float.class);
        Constructor<?> vecCtor = vec.getConstructor(double.class, double.class, double.class);
        Constructor<?> bufCtor = buf.getConstructor(ByteBuf.class, Class.forName("jr"));
        Method rotationSet = relative.getMethod("a", boolean.class, boolean.class);
        Method withRotation = pmr.getMethod("a", float.class, float.class);
        Method absolute = pmr.getMethod("a", pmr, pmr, Set.class);
        Method yRot = pmr.getMethod("c"), xRot = pmr.getMethod("d");
        Object codec = packetType.getField("a").get(null);
        float[][] befores = {{80f, -40f}, {-405.123f, 35.57f}};
        float[][] changes = {{130f, 120f}, {23.765f, -22.12f}};
        JsonArray scenarios = new JsonArray();
        for (int s = 0; s < befores.length; s++) {
            Object before = pmrCtor.newInstance(vecCtor.newInstance(-1.25, 64.125, 12.75),
                vecCtor.newInstance(.13, -.08, .27), befores[s][0], befores[s][1]);
            JsonObject scenario = new JsonObject();
            scenario.add("before", pair(befores[s][0], befores[s][1]));
            scenario.add("change", pair(changes[s][0], changes[s][1]));
            JsonArray cases = new JsonArray();
            for (int bits = 0; bits < 4; bits++) {
                boolean relY = (bits & 1) != 0, relX = (bits & 2) != 0;
                Object packet = packetCtor.newInstance(changes[s][0], relY, changes[s][1], relX);
                ByteBuf raw = Unpooled.buffer();
                try {
                    stream.getMethod("encode", Object.class, Object.class).invoke(codec, bufCtor.newInstance(raw, registries), packet);
                    byte[] bytes = new byte[raw.readableBytes()];
                    raw.getBytes(raw.readerIndex(), bytes);
                    Object decoded = stream.getMethod("decode", Object.class).invoke(codec, bufCtor.newInstance(raw, registries));
                    if (raw.isReadable() || !decoded.equals(packet)) throw new IllegalStateException("round trip");
                    Object result = absolute.invoke(null, before,
                        withRotation.invoke(before, changes[s][0], changes[s][1]), rotationSet.invoke(null, relY, relX));
                    JsonObject c = new JsonObject();
                    c.addProperty("relative_yaw", relY);
                    c.addProperty("relative_pitch", relX);
                    c.addProperty("hex", HexFormat.of().formatHex(bytes));
                    c.add("expected", pair((float) yRot.invoke(result), (float) xRot.invoke(result)));
                    cases.add(c);
                } finally { raw.release(); }
            }
            scenario.add("cases", cases);
            scenarios.add(scenario);
        }
        Files.writeString(Path.of(args[0]), new GsonBuilder().setPrettyPrinting().create().toJson(scenarios) + "\n");
        System.out.println("8 native player rotation encodings and resolutions exported");
    }
}
