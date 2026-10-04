// Own read-only JDK-tool wrapper; no game code is copied or replaced.
import java.io.PrintWriter;
import java.util.spi.ToolProvider;

public class InspectNativeClose {
    public static void main(String[] args) {
        if (args.length != 2) {
            throw new IllegalArgumentException("expected original native classpath and class name");
        }
        var tool = ToolProvider.findFirst("javap").orElseThrow();
        int result = tool.run(new PrintWriter(System.out, true), new PrintWriter(System.err, true),
                "-classpath", args[0], "-c", "-p", args[1]);
        if (result != 0) {
            throw new IllegalStateException("native bytecode inspection failed");
        }
    }
}
