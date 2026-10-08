// Own direct caller of the original 1.16.1 ResourceLocation constructor/getters.
import com.google.gson.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
public final class ExportResourceIdentifiers {
    public static void main(String[] args) throws Exception {
        try(PrintWriter out=new PrintWriter(Files.newBufferedWriter(Path.of(args[1]).getParent().resolve("original-legacy-identifiers-bytecode.log")))) {
            int result=java.util.spi.ToolProvider.findFirst("javap").orElseThrow().run(out,out,"-classpath",System.getProperty("java.class.path"),"-p","-c","uh");
            if(result!=0)throw new IllegalStateException("own original legacy identifier inspection failed");
        }
        Class<?> identifier=Class.forName("uh");JsonArray rows=new JsonArray();
        for(JsonElement element:new JsonParser().parse(Files.readString(Path.of(args[0]))).getAsJsonArray()) {
            String input=element.getAsString();JsonObject row=new JsonObject();row.addProperty("input",input);
            try {
                Object value=identifier.getConstructor(String.class).newInstance(input);row.addProperty("accepted",true);
                row.addProperty("namespace",(String)identifier.getMethod("b").invoke(value));row.addProperty("path",(String)identifier.getMethod("a").invoke(value));
            } catch(InvocationTargetException failure) {
                Throwable original=failure.getCause();if(original instanceof Error error)throw error;
                row.addProperty("accepted",false);row.addProperty("failure",original.getClass().getName()+": "+original.getMessage());
            }
            rows.add(row);
        }
        String json=new GsonBuilder().setPrettyPrinting().create().toJson(rows);StringBuilder safe=new StringBuilder();
        for(int i=0;i<json.length();i++) {
            char c=json.charAt(i);if(Character.isSurrogate(c))safe.append(String.format("\\u%04x",(int)c));else safe.append(c);
        }
        Files.writeString(Path.of(args[1]),safe+"\n");
        System.out.println("original 1.16.1 ResourceLocation cases="+rows.size());
    }
}
