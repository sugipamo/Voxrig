// Pure native outline/view-vector oracle for the SHA-1-pinned official 1.16.1 JAR.
// No server or world starts. Official mappings pin all obfuscated method names.
import com.google.gson.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;

public final class VerifyLegacyTargeting {
    static JsonArray array(double... values) {
        var out = new JsonArray();
        for (double value : values) out.add(value);
        return out;
    }
    static JsonArray cell(fu p) {
        var out = new JsonArray();
        out.add(p.u()); out.add(p.v()); out.add(p.w());
        return out;
    }
    static JsonArray vector(dem v) { return array(v.b, v.c, v.d); }
    static Object uninitialized(Class<?> type) throws Exception {
        var cls = Class.forName("sun.misc.Unsafe");
        var field = cls.getDeclaredField("theUnsafe");
        field.setAccessible(true);
        return cls.getMethod("allocateInstance", Class.class).invoke(field.get(null), type);
    }
    static void field(Object object, String name, Object value) throws Exception {
        var field = object.getClass().getDeclaredField(name);
        field.setAccessible(true);
        field.set(object, value);
    }
    static bpj context(dem start, dem end) throws Exception {
        // 1.16.1 has no public empty-entity constructor. Populate only this native
        // data carrier with the same native OUTLINE/NONE/empty collision context.
        // Native BlockGetter.clip, VoxelShape.clip and traversal run unmodified.
        var result = (bpj)uninitialized(bpj.class);
        field(result, "a", start);
        field(result, "b", end);
        field(result, "c", bpj.a.b); // OUTLINE.
        field(result, "d", bpj.b.a); // NONE (no fluid targeting).
        field(result, "e", der.a());
        return result;
    }
    static final class World implements bpg {
        final Map<fu, cfj> cells = new HashMap<>();
        final cfj air = gl.aj.a(new uh("minecraft", "air")).n();
        public cdl c(fu position) { return null; }
        public cfj d_(fu position) { return cells.getOrDefault(position, air); }
        public cxa b(fu position) { return d_(position).m(); }
    }
    static JsonArray boxes(dfg shape) {
        var result = new JsonArray();
        for (var box : shape.d()) result.add(array(box.a, box.b, box.c, box.d, box.e, box.f));
        return result;
    }
    public static void main(String[] args) throws Exception {
        if (!u.a().getName().equals("1.16.1")) throw new IllegalStateException("version");
        uj.a();
        try {
            var calculateViewVector = aom.class.getDeclaredMethod("c", float.class, float.class);
            calculateViewVector.setAccessible(true);
            // calculateViewVector uses no instance fields; avoid world-dependent construction.
            var entity = uninitialized(bay.class);
            var rotations = new JsonArray();
            for (float yaw : new float[]{0, 90, -90, 180, -180, 35.57f, -405.123f, 359.99f, 1000000f}) {
                for (float pitch : new float[]{-90, -45, -.1f, 0, .1f, 48.366f, 89.999f, 90}) {
                    var record = new JsonObject();
                    record.add("rotation", array(yaw, pitch));
                    record.add("direction", vector((dem)calculateViewVector.invoke(entity, pitch, yaw)));
                    rotations.add(record);
                }
            }
            var outlines = new JsonArray();
            String[] names = {"stone", "dirt", "grass_block", "cobblestone", "oak_planks", "spruce_planks",
                "quartz_block", "smooth_quartz", "white_concrete", "glass", "andesite", "granite"};
            for (String name : names) {
                var block = gl.aj.a(new uh("minecraft", name));
                if (!gl.aj.b(block).toString().equals("minecraft:" + name))
                    throw new IllegalStateException("registry fallback for " + name);
                for (var state : block.m().a()) {
                    var record = new JsonObject();
                    record.addProperty("name", "minecraft:" + name);
                    record.addProperty("native_state", state.toString());
                    record.add("outline", boxes(state.a(bpp.a, fu.b, der.a())));
                    record.add("auxiliary", boxes(state.m(bpp.a, fu.b)));
                    outlines.add(record);
                }
            }
            var rays = new JsonArray();
            var cube = gl.aj.a(new uh("minecraft", "stone")).n();
            var sets = List.of(
                List.<fu>of(), List.of(new fu(0, 0, 0)),
                List.of(new fu(0, 0, 0), new fu(1, 0, 0)),
                List.of(new fu(0, 0, 0), new fu(0, 1, 0), new fu(0, 0, 1)),
                List.of(new fu(-1, 0, 0), new fu(0, 0, -1)));
            var starts = List.of(new dem(.5, 2, .5), new dem(.5, -1, .5),
                new dem(-1, .5, .5), new dem(2, .5, .5), new dem(.5, .5, -1),
                new dem(.5, .5, 2), new dem(-1, -1, -1), new dem(2, 2, 2),
                new dem(.5, .5, .5), new dem(0, 1, 0), new dem(1, 1, 1),
                new dem(-1, 1.000000099, .5), new dem(-1, 1.000000101, .5));
            var ends = List.of(new dem(.5, .5, .5), new dem(2, 2, 2),
                new dem(-1, -1, -1), new dem(2, .5, .5), new dem(.5, -1, .5),
                new dem(.5, .5, 2), new dem(0, 0, 0), new dem(1, 1, 1),
                new dem(.50000001, .5, .5));
            for (var cells : sets) {
                var world = new World();
                var geometry = new JsonArray();
                for (var p : cells) {
                    world.cells.put(p, cube);
                    geometry.add(cell(p));
                }
                for (var start : starts) for (var end : ends) {
                    var record = new JsonObject();
                    record.add("cells", geometry);
                    record.add("start", vector(start));
                    record.add("end", vector(end));
                    var hit = world.a(context(start, end)); // BlockGetter.clip.
                    if (hit.c().name().equals("BLOCK")) {
                        var expected = new JsonObject();
                        var p = hit.a();
                        expected.add("position", cell(p));
                        expected.addProperty("face", hit.b().name().toLowerCase(Locale.ROOT));
                        expected.add("point", vector(hit.e()));
                        record.add("hit", expected);
                    } else {
                        record.add("hit", JsonNull.INSTANCE);
                    }
                    rays.add(record);
                }
            }
            var result = new JsonObject();
            result.add("rotations", rotations);
            result.add("outlines", outlines);
            result.add("rays", rays);
            Files.writeString(Path.of(args[0]), new Gson().toJson(result) + "\n");
            System.out.println(rotations.size() + " rotations, " + outlines.size() + " outlines, " + rays.size() + " native ray results exported");
        } finally {
            v.h();
        }
    }
}
