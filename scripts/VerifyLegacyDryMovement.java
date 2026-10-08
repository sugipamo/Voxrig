// Native Java 1.16.1 methods, with names pinned by the official server mappings.
// Run with the unmodified, SHA-1-verified official 1.16.1 server.jar on the classpath.
// No server/world is started; this exports independent movement primitives.
import com.google.gson.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;

public final class VerifyLegacyDryMovement {
    static JsonArray array(double... values) {
        var out = new JsonArray();
        for (double value : values) out.add(value);
        return out;
    }
    static JsonArray vector(dem value) { return array(value.b, value.c, value.d); }
    static deg body(double[] position, aon dimensions) {
        double half = dimensions.a / 2.0f;
        return new deg(position[0] - half, position[1], position[2] - half,
            position[0] + half, position[1] + dimensions.b, position[2] + half);
    }
    public static void main(String[] args) throws Exception {
        if (!u.a().getName().equals("1.16.1")) throw new IllegalStateException("version");
        uj.a(); // Bootstrap.bootStrap; no network, server or world.
        var standingField = bec.class.getDeclaredField("bo");
        standingField.setAccessible(true);
        var dimensions = (aon)standingField.get(null);
        var inputVector = aom.class.getDeclaredMethod("a", dem.class, float.class, float.class);
        inputVector.setAccessible(true); // Entity.getInputVector.
        var inputs = new JsonArray();
        for (int strafe = -1; strafe <= 1; strafe++) {
            for (int forward = -1; forward <= 1; forward++) {
                for (float yaw : new float[]{0, 90, -90, 180, 35.57f, -405.123f}) {
                    // The bounded consumer supplies digital inputs scaled by .98f.
                    var input = new dem(strafe * .98f, 0, forward * .98f);
                    for (float speed : new float[]{.1f * (.21600002f / (.6f * .6f * .6f)), .02f}) {
                        var value = (dem)inputVector.invoke(null, input, speed, yaw);
                        var record = new JsonObject();
                        record.addProperty("strafe", strafe);
                        record.addProperty("forward", forward);
                        record.addProperty("yaw", yaw);
                        record.addProperty("speed", speed);
                        record.add("expected", vector(value));
                        inputs.add(record);
                    }
                }
            }
        }
        var collisions = new JsonArray();
        var sets = List.of(
            List.of(new deg(0, 0, 0, 1, 1, 1)),
            List.of(new deg(0, 0, 0, 1, 1, 1), new deg(1, 1, 0, 2, 2, 1)),
            List.of(new deg(0, 0, 0, 1, 1, 1), new deg(0, 2, 0, 1, 3, 1)),
            List.of(new deg(1, 1, 0, 2, 2, 1), new deg(0, 1, 1, 1, 2, 2)));
        for (var boxes : sets) {
            for (double[] position : List.of(new double[]{.5, 1, .5}, new double[]{.5, 1.2, .5}, new double[]{1.29999999, 1, .5})) {
                for (var motion : List.of(new dem(.4, -.08, .2), new dem(.2, -.08, .4), new dem(0, .42, 0), new dem(0, -.4, 0), new dem(-.4, 0, -.4), new dem(1e-8, -1e-8, 0))) {
                    var shapeStream = boxes.stream().map(dfd::a);
                    // Entity.collideBoundingBoxLegacy calls native voxel-shape collision.
                    var adjusted = aom.a(motion, body(position, dimensions), new aee<>(shapeStream));
                    var record = new JsonObject();
                    record.add("position", array(position));
                    record.add("motion", vector(motion));
                    record.add("expected", vector(adjusted));
                    var geometry = new JsonArray();
                    for (var box : boxes) geometry.add(array(box.a, box.b, box.c, box.d, box.e, box.f));
                    record.add("boxes", geometry);
                    collisions.add(record);
                }
            }
        }
        var materials = new JsonObject();
        var materialShapes = new JsonObject();
        String[] names = {"stone", "dirt", "grass_block", "cobblestone", "oak_planks", "spruce_planks",
            "quartz_block", "smooth_quartz", "white_concrete", "glass", "andesite", "granite"};
        // Resolve by registry name, rather than relying on a second field-name table.
        for (String name : names) {
            var block = gl.aj.a(new uh("minecraft", name));
            if (!gl.aj.b(block).toString().equals("minecraft:" + name))
                throw new IllegalStateException("registry fallback for " + name);
            materials.add("minecraft:" + name, array(block.j(), block.k(), block.l()));
            var shape = new JsonArray();
            for (var box : block.n().b(bpp.a, fu.b, der.a()).d())
                shape.add(array(box.a, box.b, box.c, box.d, box.e, box.f));
            materialShapes.add("minecraft:" + name, shape);
        }
        var result = new JsonObject();
        result.add("standing_dimensions", array(dimensions.a, dimensions.b));
        result.add("inputs", inputs);
        result.add("collisions", collisions);
        result.add("materials", materials);
        result.add("material_shapes", materialShapes);
        Files.writeString(Path.of(args[0]), new Gson().toJson(result) + "\n");
        System.out.println(inputs.size() + " input/rotation and " + collisions.size() + " collision results exported");
        v.h(); // Shut down Bootstrap-created worker executors.
    }
}
