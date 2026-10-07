// Own test launcher/control only. Calls unchanged original server lifecycle/codec methods.
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import java.util.concurrent.*;
import java.util.concurrent.atomic.AtomicReference;

public final class NativeReconfigurationControl {
    static final Path request = Path.of("voxrig-control-request.txt");
    static final Path reply = Path.of("voxrig-control-reply.txt");
    static final Map<String, Object> pending = new HashMap<>();
    static Field field(Class<?> type, String name) throws Exception {
        for (Class<?> c = type; c != null; c = c.getSuperclass()) {
            try { Field f = c.getDeclaredField(name); f.setAccessible(true); return f; }
            catch (NoSuchFieldException ignored) {}
        }
        throw new NoSuchFieldException(name);
    }
    static Object call(Object o, String name) throws Exception {
        return o.getClass().getMethod(name).invoke(o);
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
            for (Field f : target.getClass().getDeclaredFields()) {
                f.setAccessible(true);
                if (f.get(target) instanceof AtomicReference<?> ref) {
                    Object candidate = ref.get();
                    for (Class<?> c = candidate.getClass(); c != null; c = c.getSuperclass())
                        if (c.getName().equals("net.minecraft.server.MinecraftServer")) return candidate;
                }
            }
        }
        return null;
    }
    static String action(Object server, String[] parts) throws Exception {
        String command = parts[1], name = parts[2];
        if (!Set.of("UnifiedProbe", "ManagedPeer").contains(name)) throw new IllegalArgumentException("fixture identity required");
        if (command.equals("start")) {
            Object connection = Objects.requireNonNull(pending.get(name), "no pending switch");
            Object listener = call(connection, "k");
            if (!listener.getClass().getName().equals("ayg")) throw new IllegalStateException("native configuration ACK pending");
            call(listener, "l"); // Original startConfiguration, original registry/task pipeline.
            pending.remove(name);
            return "ayg.startConfiguration";
        }
        Object players = call(server, "aj");
        Object player = players.getClass().getMethod("c", String.class).invoke(players, name);
        Object listener = field(player.getClass(), "g").get(player);
        if (!listener.getClass().getName().equals("ayi")) throw new IllegalStateException("native play listener required");
        if (command.equals("switch")) {
            if (pending.containsKey(name)) throw new IllegalStateException("switch already pending");
            Object connection = field(listener.getClass(), "e").get(listener);
            call(listener, "o"); // Original switchToConfig, including actual player departure.
            pending.put(name, connection);
            return "ayi.switchToConfig";
        }
        if (command.equals("tab")) {
            ClassLoader loader = listener.getClass().getClassLoader();
            Class<?> text = Class.forName("yh", true, loader);
            Object header = text.getMethod("b", String.class).invoke(null, "ContextHeader");
            Object footer = text.getMethod("b", String.class).invoke(null, "ContextFooter");
            Object packet = Class.forName("ahl", true, loader).getConstructor(text, text).newInstance(header, footer);
            listener.getClass().getMethod("b", Class.forName("aay", true, loader)).invoke(listener, packet);
            return "original ClientboundTabListPacket";
        }
        throw new IllegalArgumentException("unknown control action");
    }
    static void control() {
        String seen = null;
        try {
            Object server;
            while ((server = server()) == null) Thread.sleep(100);
            Object actual = server;
            System.out.println("VOXRIG_NATIVE_CONTROL_READY");
            while (true) {
                if (!Files.exists(request)) { Thread.sleep(25); continue; }
                String line = Files.readString(request);
                if (line.equals(seen)) { Thread.sleep(25); continue; }
                seen = line;
                String[] parts = line.strip().split("\\t");
                if (parts.length != 3 || line.length() > 128) throw new IllegalArgumentException("invalid fixture request");
                CompletableFuture<String> completed = new CompletableFuture<>();
                ((Executor) actual).execute(() -> {
                    try { completed.complete(action(actual, parts)); }
                    catch (Exception error) { completed.completeExceptionally(error); }
                });
                String result;
                try { result = "ok\t" + completed.get(15, TimeUnit.SECONDS); }
                catch (Exception error) { result = "error\t" + error.getClass().getName() + ":" + error.getMessage(); }
                Files.writeString(reply, parts[0] + "\t" + result + "\n");
            }
        } catch (Exception error) {
            error.printStackTrace();
            System.out.println("VOXRIG_NATIVE_CONTROL_FAILED");
        }
    }
    public static void main(String[] args) throws Exception {
        Thread control = new Thread(NativeReconfigurationControl::control, "Voxrig native configuration control");
        control.setDaemon(true); control.start();
        // Bundler and original Main are unmodified; only this own control observes the server reference.
        Class.forName("net.minecraft.bundler.Main").getMethod("main", String[].class).invoke(null, (Object) args);
    }
}
