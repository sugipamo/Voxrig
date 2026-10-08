// Own fixture launcher. Invoke unchanged official protocol-736 server methods;
// no server class rewriting, packet injection, filtering or generated codec.
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.concurrent.*;
import java.util.concurrent.atomic.AtomicReference;

public final class NativeReaderStressControl {
    static Field field(Class<?> type, String name) throws Exception {
        for (Class<?> c = type; c != null; c = c.getSuperclass()) {
            try { Field f = c.getDeclaredField(name); f.setAccessible(true); return f; }
            catch (NoSuchFieldException ignored) {}
        }
        throw new NoSuchFieldException(name);
    }
    static Object server() throws Exception {
        for (Thread thread : Thread.getAllStackTraces().keySet()) {
            if (!thread.getName().equals("Server thread")) continue;
            Object owner = thread;
            try { owner = field(Thread.class, "holder").get(thread); }
            catch (NoSuchFieldException ignored) {}
            Object target;
            try { target = field(owner.getClass(), "task").get(owner); }
            catch (NoSuchFieldException ignored) { target = field(owner.getClass(), "target").get(owner); }
            if (target == null) continue;
            for (Field f : target.getClass().getDeclaredFields()) {
                f.setAccessible(true);
                if (f.get(target) instanceof AtomicReference<?> ref) {
                    Object candidate = ref.get();
                    if (candidate == null) continue;
                    for (Class<?> c = candidate.getClass(); c != null; c = c.getSuperclass())
                        if (c.getName().equals("net.minecraft.server.MinecraftServer")) return candidate;
                }
            }
        }
        return null;
    }
    static void install() {
        try {
            Object server;
            while ((server = server()) == null) Thread.sleep(100);
            Object actual = server;
            Class<?> entity = Class.forName("aom");
            Class<?> level = Class.forName("zd");
            Method levels = actual.getClass().getMethod("F"); // getAllLevels
            Method entities = level.getMethod("z"); // getAllEntities
            Method broadcast = level.getMethod("a", entity, byte.class); // broadcastEntityEvent
            Runnable tick = () -> {
                if (!Files.exists(Path.of("voxrig-reader-stress-enabled"))) return;
                try {
                    int wolves = 0;
                    for (Object world : (Iterable<?>) levels.invoke(actual)) {
                        for (Object e : (Iterable<?>) entities.invoke(world)) {
                            if (!e.getClass().getName().equals("azk")) continue;
                            if (++wolves > 6) throw new IllegalStateException("fixture has more than six wolves");
                            for (int i = 0; i < 2; i++) broadcast.invoke(world, e, (byte) 8);
                        }
                    }
                } catch (Exception error) { throw new RuntimeException("native fixture failed", error); }
            };
            ((Executor) actual).execute(() -> {
                try {
                    actual.getClass().getMethod("b", Runnable.class).invoke(actual, tick); // addTickable
                    System.out.println("VOXRIG_NATIVE_READER_STRESS_READY");
                } catch (Exception error) { throw new RuntimeException(error); }
            });
        } catch (Exception error) { error.printStackTrace(); }
    }
    public static void main(String[] args) throws Exception {
        Thread control = new Thread(NativeReaderStressControl::install, "Voxrig reader fixture control");
        control.setDaemon(true); control.start();
        Class.forName("net.minecraft.server.Main").getMethod("main", String[].class).invoke(null, (Object) args);
    }
}
