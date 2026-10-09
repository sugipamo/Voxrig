import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.lang.reflect.Constructor;
import java.lang.reflect.Method;

/** Execute original mapped storage constructors/getters. Never a game server. */
public final class VerifyReceivedHeightmap {
    public static void main(String[] args) throws Exception {
        Class<?> storage = Class.forName(args[0].equals("1.16.1")
            ? "net.minecraft.util.BitStorage" : "net.minecraft.util.SimpleBitStorage");
        Constructor<?> constructor = storage.getConstructor(int.class, int.class, long[].class);
        Method get = storage.getMethod("get", int.class);
        BufferedReader input = new BufferedReader(new InputStreamReader(System.in));
        String line;
        while ((line = input.readLine()) != null) {
            String[] fields = line.split(" ");
            int height = Integer.parseInt(fields[0]);
            int minimum = Integer.parseInt(fields[1]);
            long[] words = new long[fields.length - 2];
            for (int i = 0; i < words.length; i++) words[i] = Long.parseLong(fields[i + 2]);
            // Legacy Heightmap fixes nine bits. Modern Heightmap uses
            // ceilLog2(chunk height + 1); the original constructor/getter owns
            // array shape, padding and value extraction.
            int bits = args[0].equals("1.16.1") ? 9
                : 32 - Integer.numberOfLeadingZeros(height);
            Object values = constructor.newInstance(bits, 256, words);
            StringBuilder result = new StringBuilder("[");
            for (int i = 0; i < 256; i++) {
                if (i > 0) result.append(',');
                result.append(minimum + (Integer)get.invoke(values, i));
            }
            result.append(']');
            System.out.println(result);
        }
    }
}
