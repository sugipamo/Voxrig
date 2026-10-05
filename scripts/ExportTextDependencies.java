// Own observer of unchanged text dependency constructors, getters and equals.
import com.google.gson.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
public final class ExportTextDependencies {
    static JsonObject describe(Object value) throws Exception {
        JsonObject out = ExportTextCore.describe(value);
        Object style = Class.forName("yh").getMethod("a").invoke(value);
        Object hover = ExportTextCore.field(style, "k"), click = ExportTextCore.field(style, "j");
        JsonObject fields = out.getAsJsonObject("style");
        if (hover != null && hover.getClass().getName().equals("yo$d")) {
            Object item = Class.forName("yo$d").getMethod("b").invoke(hover);
            fields.addProperty("hover_item_hex", ExportItemProperties.encoded(item));
            fields.add("hover_item_properties", ExportItemProperties.properties(item));
        }
        if (click != null && click.getClass().getName().equals("yf$h")) {
            Object dialog = ExportTextCore.field(click, "c");
            fields.addProperty("dialog_holder_class", dialog.getClass().getName());
            if (dialog.getClass().getName().equals("jd$c")) {
                Object key = Class.forName("jd$c").getMethod("h").invoke(dialog);
                fields.addProperty("dialog_name", Class.forName("amt").getMethod("a").invoke(key).toString());
            } else {
                if (!Class.forName("jd").isInstance(dialog)) throw new IllegalStateException("own unexpected native dialog holder " + dialog.getClass().getName());
                fields.addProperty("dialog_inline_class", Class.forName("jd").getMethod("a").invoke(dialog).getClass().getName());
            }
        }
        JsonArray siblings = new JsonArray();
        for (Object child : (List<?>) Class.forName("yh").getMethod("c").invoke(value)) siblings.add(describe(child));
        out.add("siblings", siblings);
        return out;
    }
    public static void main(String[] args) throws Exception {
        ExportItemProperties.init("1.21.11");
        Object type = null;
        for (Object candidate : (Iterable<?>) ExportItemProperties.components)
            if (ExportItemProperties.name.invoke(ExportItemProperties.components, candidate).toString().equals("minecraft:custom_name")) type = candidate;
        Object codec = Class.forName("kh").getMethod("f").invoke(type);
        JsonArray cases = new JsonArray(); List<Object> values = new ArrayList<>();
        for (JsonElement element : JsonParser.parseString(Files.readString(Path.of(args[0]))).getAsJsonArray()) {
            JsonObject request = element.getAsJsonObject(), row = request.deepCopy();
            try {
                Object value = ExportComponentValueRules.decode(codec, HexFormat.of().parseHex(request.get("input_hex").getAsString()));
                row.addProperty("accepted", true); row.add("fields", describe(value));
                try { row.addProperty("canonical_hex", HexFormat.of().formatHex(ExportItemComponents.roundtrip(codec, value))); }
                catch (InvocationTargetException failure) { row.addProperty("encode_failure", failure.getCause().getClass().getName()); }
                values.add(value);
            } catch (InvocationTargetException failure) {
                Throwable original = ExportComponentNormalization.cause(failure); if (original instanceof Error error) throw error;
                row.addProperty("accepted", false); row.addProperty("failure", original.getClass().getName()+": "+original.getMessage()); values.add(null);
            }
            cases.add(row);
        }
        JsonArray pairs = new JsonArray();
        for (int a=0; a<values.size(); a++) for (int b=a; b<values.size(); b++) if (values.get(a)!=null && values.get(b)!=null) {
            JsonObject pair = new JsonObject(); pair.addProperty("a", a); pair.addProperty("b", b); pair.addProperty("equal", values.get(a).equals(values.get(b))); pairs.add(pair);
        }
        JsonObject out = new JsonObject(); out.addProperty("java_version", System.getProperty("java.version")); out.add("cases",cases); out.add("pairs",pairs);
        Files.writeString(Path.of(args[1]), ExportComponentNormalization.jsonUtf16(new GsonBuilder().serializeNulls().create().toJson(out)));
        Class.forName("bas").getMethod("close").invoke(ExportItemComponents.resourceManager);
        System.out.println("original text dependency cases="+cases.size()+" pairs="+pairs.size());
    }
}
