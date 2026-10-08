// Own reflective export of Java 1.21.11 entity-type dimensions from the official server.
// Names follow the published 1.21.11 server mappings (sha1 5621e925...):
// SharedConstants=w.a(), Bootstrap=amv.a(), BuiltInRegistries.ENTITY_TYPE=mi.g,
// Registry.getKey=jq.b, EntityType.dimensions=cgu.cv, EntityDimensions width/height/eyeHeight=cgn.a/b/c,
// DefaultAttributes.hasSupplier=cit.b (true exactly for living entity types).
package voxrig.export;

import java.lang.reflect.*;
import java.util.*;

public final class ExportEntityDimensions {
    static Object field(Object owner, Class<?> type, String name) throws Exception {
        Field f = type.getDeclaredField(name);
        f.setAccessible(true);
        return f.get(owner);
    }

    public static void main(String[] args) throws Exception {
        Class.forName("w").getMethod("a").invoke(null);
        Class.forName("amv").getMethod("a").invoke(null);
        Object registry = Class.forName("mi").getField("g").get(null);
        Method getKey = Class.forName("jq").getMethod("b", Object.class);
        Class<?> entityType = Class.forName("cgu");
        Class<?> dimensions = Class.forName("cgn");
        Method living = Class.forName("cit").getMethod("b", entityType);
        TreeMap<String, String> rows = new TreeMap<>();
        for (Object type : (Iterable<?>) registry) {
            Object d = field(type, entityType, "cv");
            rows.put(getKey.invoke(registry, type).toString(), String.format(Locale.ROOT,
                "{\"width\":%s,\"height\":%s,\"eye_height\":%s,\"living\":%s}",
                Float.toString((float) field(d, dimensions, "a")),
                Float.toString((float) field(d, dimensions, "b")),
                Float.toString((float) field(d, dimensions, "c")),
                living.invoke(null, type)));
        }
        StringBuilder out = new StringBuilder("{\"minecraft\":\"1.21.11\",\"entities\":{");
        boolean first = true;
        for (Map.Entry<String, String> row : rows.entrySet()) {
            if (!first) out.append(',');
            first = false;
            out.append('"').append(row.getKey()).append("\":").append(row.getValue());
        }
        System.out.println(out.append("}}"));
    }
}
