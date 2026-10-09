// Own reflective export of per-block physics factors from an official server.
// Obfuscated names come from Mojang's published server mappings for that version
// and are passed as arguments, so one source serves every version:
//   version detectClass|- bootstrapClass registryOwner registryField
//   registryClass getKeyMethod behaviourClass frictionField speedField jumpField
//   1.16.1  (mappings sha1 11120c39...): 1.16.1 - uj gl aj gl b cfi ax ay az
//   1.21.11 (mappings sha1 5621e925...): 1.21.11 w amv mi e jq b eog J K L
package voxrig.export;

import java.lang.reflect.*;
import java.util.*;

public final class ExportBlockPhysics {
    static float field(Object owner, Class<?> type, String name) throws Exception {
        Field f = type.getDeclaredField(name);
        f.setAccessible(true);
        return f.getFloat(owner);
    }

    public static void main(String[] a) throws Exception {
        if (!a[1].equals("-")) Class.forName(a[1]).getMethod("a").invoke(null);
        Class.forName(a[2]).getMethod("a").invoke(null);
        Object registry = Class.forName(a[3]).getField(a[4]).get(null);
        Method getKey = Class.forName(a[5]).getMethod(a[6], Object.class);
        Class<?> behaviour = Class.forName(a[7]);
        TreeMap<String, String> rows = new TreeMap<>();
        for (Object block : (Iterable<?>) registry) {
            rows.put(getKey.invoke(registry, block).toString(), String.format(Locale.ROOT,
                "{\"friction\":%s,\"speed_factor\":%s,\"jump_factor\":%s}",
                Float.toString(field(block, behaviour, a[8])),
                Float.toString(field(block, behaviour, a[9])),
                Float.toString(field(block, behaviour, a[10]))));
        }
        StringBuilder out = new StringBuilder("{\"minecraft\":\"" + a[0] + "\",\"blocks\":{");
        boolean first = true;
        for (Map.Entry<String, String> row : rows.entrySet()) {
            if (!first) out.append(',');
            first = false;
            out.append('"').append(row.getKey()).append("\":").append(row.getValue());
        }
        System.out.println(out.append("}}"));
    }
}
