// Own reflective export of every synched entity-data accessor in an official server jar.
// Obfuscated names come from the published server mappings and are passed as arguments, so
// the same tool serves every version; scripts/generate_entity_data.py maps them back.
//
// java -cp <server classpath> ExportEntityData.java <jar> <SharedConstants.init|-> \
//     <Bootstrap.bootStrap> <Entity> <EntityType> <EntityDataAccessor> <EntityDataAccessor.id> \
//     <legacy|modern> <Entity.defineSynchedData> <SynchedEntityData or its Builder> \
//     <its itemsById field> <DataItem> <DataItem.accessor> <DataItem initial value field> \
//     <Entity.getMaxAirSupply> [<Entity.entityData> (legacy)]
//
// Defaults come from allocating each entity type's class without running a constructor and
// calling defineSynchedData on a fresh data holder: legacy (1.16.1) defines into
// Entity.entityData, modern (1.21.11) into a SynchedEntityData.Builder.
//
// Output lines: "type <EntityType field> <entity class>", "class <class> <superclass>",
// "data <class> <field> <id>", "default <entity class> <id> <kind>:<value>" and
// "partial <entity class> <exception>" when defineSynchedData stopped early, and
// "air <entity class> <getMaxAirSupply()>". The Entity constructor defines the base fields
// before defineSynchedData; their constant defaults are added by the generator, except the
// air supply, whose default is the type's getMaxAirSupply().
package voxrig.export;

import java.lang.reflect.*;
import java.util.*;
import java.util.jar.*;

public final class ExportEntityData {
    static void call(String spec) throws Exception {
        if (spec.equals("-")) return;
        int dot = spec.lastIndexOf('.');
        Method m = Class.forName(spec.substring(0, dot)).getDeclaredMethod(spec.substring(dot + 1));
        m.setAccessible(true);
        m.invoke(null);
    }

    static String render(Object v) {
        if (v instanceof Byte b) return "byte:" + b;
        if (v instanceof Integer i) return "int:" + i;
        if (v instanceof Long l) return "long:" + l;
        if (v instanceof Float f) return "float:" + f;
        if (v instanceof Boolean z) return "bool:" + z;
        return "other:-";
    }

    public static void main(String[] args) throws Exception {
        // Bootstrap redirects System.out into the server logger.
        java.io.PrintStream out = System.out;
        call(args[1]);
        call(args[2]);
        Class<?> entity = Class.forName(args[3]);
        Class<?> entityType = Class.forName(args[4]);
        Class<?> accessor = Class.forName(args[5]);
        Field id = accessor.getDeclaredField(args[6]);
        id.setAccessible(true);
        for (Field f : entityType.getDeclaredFields()) {
            if (!Modifier.isStatic(f.getModifiers()) || f.getType() != entityType) continue;
            if (f.getGenericType() instanceof ParameterizedType p
                    && p.getActualTypeArguments()[0] instanceof Class<?> c) {
                out.println("type " + f.getName() + " " + c.getName());
            }
        }
        TreeSet<String> rows = new TreeSet<>();
        List<Class<?>> loaded = new ArrayList<>();
        try (JarFile jar = new JarFile(args[0])) {
            for (JarEntry e : Collections.list(jar.entries())) {
                String n = e.getName();
                if (!n.endsWith(".class") || n.contains("/") && !n.startsWith("net/minecraft/")) continue;
                Class<?> c;
                try {
                    c = Class.forName(n.substring(0, n.length() - 6).replace('/', '.'), false,
                        ExportEntityData.class.getClassLoader());
                } catch (Throwable t) {
                    continue;
                }
                if (!entity.isAssignableFrom(c)) continue;
                loaded.add(c);
                if (c != entity) rows.add("class " + c.getName() + " " + c.getSuperclass().getName());
                for (Field f : c.getDeclaredFields()) {
                    if (!Modifier.isStatic(f.getModifiers()) || f.getType() != accessor) continue;
                    f.setAccessible(true);
                    rows.add("data " + c.getName() + " " + f.getName() + " " + id.get(f.get(null)));
                }
            }
        }
        rows.forEach(out::println);
        TreeSet<String> defaults = new TreeSet<>();
        Field unsafeField = Class.forName("sun.misc.Unsafe").getDeclaredField("theUnsafe");
        unsafeField.setAccessible(true);
        Object unsafe = unsafeField.get(null);
        Method allocate = unsafe.getClass().getMethod("allocateInstance", Class.class);
        boolean legacy = args[7].equals("legacy");
        Class<?> holder = Class.forName(args[9]);
        Method define = null;
        for (Method m : entity.getDeclaredMethods()) {
            if (m.getName().equals(args[8]) && m.getParameterCount() == (legacy ? 0 : 1)
                    && (legacy || m.getParameterTypes()[0] == holder)) define = m;
        }
        define.setAccessible(true);
        Constructor<?> make = holder.getDeclaredConstructors()[0];
        for (Constructor<?> k : holder.getDeclaredConstructors()) if (k.getParameterCount() == 1) make = k;
        make.setAccessible(true);
        Field items = holder.getDeclaredField(args[10]);
        items.setAccessible(true);
        Class<?> item = Class.forName(args[11]);
        Field itemAccessor = item.getDeclaredField(args[12]);
        Field itemValue = item.getDeclaredField(args[13]);
        itemAccessor.setAccessible(true);
        itemValue.setAccessible(true);
        TreeSet<String> classes = new TreeSet<>();
        for (Field f : entityType.getDeclaredFields()) {
            if (Modifier.isStatic(f.getModifiers()) && f.getType() == entityType
                    && f.getGenericType() instanceof ParameterizedType p
                    && p.getActualTypeArguments()[0] instanceof Class<?> c) classes.add(c.getName());
        }
        for (String name : classes) {
            Class<?> c = Class.forName(name);
            // An abstract type class (Player) is defined through a concrete direct
            // subclass that adds no accessors of its own (ServerPlayer).
            if (Modifier.isAbstract(c.getModifiers())) {
                for (Class<?> sub : loaded) {
                    if (sub.getSuperclass() == c && !Modifier.isAbstract(sub.getModifiers())
                            && Arrays.stream(sub.getDeclaredFields()).noneMatch(f -> f.getType() == accessor)) c = sub;
                }
            }
            try {
                Object instance = allocate.invoke(unsafe, c);
                Object data = make.newInstance(instance);
                try {
                    if (legacy) {
                        Field slot = entity.getDeclaredField(args[15]);
                        slot.setAccessible(true);
                        slot.set(instance, data);
                        define.invoke(instance);
                    } else {
                        define.invoke(instance, data);
                    }
                } catch (InvocationTargetException partial) {
                    // Registry-backed variants need a world; earlier definitions remain.
                    defaults.add("partial " + name + " " + partial.getCause().getClass().getSimpleName());
                }
                try {
                    Method air = entity.getDeclaredMethod(args[14]);
                    air.setAccessible(true);
                    defaults.add("air " + name + " " + air.invoke(instance));
                } catch (InvocationTargetException ignored) {
                    // Leaves the air supply default unknown.
                }
                Object all = items.get(data);
                Iterable<?> values = all instanceof Map<?, ?> m ? m.values() : Arrays.asList((Object[]) all);
                for (Object v : values) {
                    if (v == null) continue;
                    defaults.add("default " + name + " " + id.get(itemAccessor.get(v)) + " " + render(itemValue.get(v)));
                }
            } catch (Throwable t) {
                defaults.add("nodefault " + name + " " + t.getClass().getSimpleName());
            }
        }
        defaults.forEach(out::println);
    }
}
