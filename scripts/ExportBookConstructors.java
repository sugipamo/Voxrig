// Own observer of unchanged book/enchantability stream constructors and getters.
import com.google.gson.*;
import java.nio.file.*;
import java.lang.reflect.*;
import java.util.*;
import java.io.*;

public class ExportBookConstructors {
 // JSON ASCII escaping belongs to this observer's output transport. Preserve
 // all original UTF-16 units, including an unpaired surrogate from text NBT.
 static String ascii(String value){StringBuilder out=new StringBuilder();for(char c:value.toCharArray()){if(c>127)out.append(String.format("\\u%04x",(int)c));else out.append(c);}return out.toString();}
 static JsonObject filtered(Object value,boolean text)throws Exception {
  RecordComponent[] components=value.getClass().getRecordComponents();
  if(components.length!=2)throw new IllegalStateException("original Filterable composition changed");
  Object raw=components[0].getAccessor().invoke(value);
  Optional<?> filtered=(Optional<?>)components[1].getAccessor().invoke(value);
  JsonObject out=new JsonObject();
  out.add("raw",text?ExportTextCore.describe(raw):new JsonPrimitive((String)raw));
  out.add("filtered",filtered.isEmpty()?JsonNull.INSTANCE:(text?ExportTextCore.describe(filtered.get()):new JsonPrimitive((String)filtered.get())));
  return out;
 }
 static JsonObject fields(String name,Object value)throws Exception {
  JsonObject out=new JsonObject();
  if(name.equals("minecraft:enchantable")){out.addProperty("value",(Integer)ExportItemComponentSchema.field(value,"c"));return out;}
  boolean written=name.equals("minecraft:written_book_content");
  if(written){out.add("title",filtered(ExportItemComponentSchema.field(value,"k"),false));out.addProperty("author",(String)ExportItemComponentSchema.field(value,"l"));out.addProperty("generation",(Integer)ExportItemComponentSchema.field(value,"m"));out.addProperty("resolved",(Boolean)ExportItemComponentSchema.field(value,"o"));}
  JsonArray pages=new JsonArray();for(Object page:(List<?>)ExportItemComponentSchema.field(value,written?"n":"g"))pages.add(filtered(page,written));out.add("pages",pages);return out;
 }
 public static void main(String[] args)throws Exception {
  Path base=Path.of(args[1]).getParent();
  try(PrintWriter log=new PrintWriter(Files.newBufferedWriter(base.resolve("original-books-bytecode.log")))){int status=java.util.spi.ToolProvider.findFirst("javap").orElseThrow().run(log,log,"-classpath",System.getProperty("java.class.path"),"-p","-c","dsm","dpk","dpl","axx");if(status!=0)throw new IllegalStateException("original book inspection failed");}
  ExportItemProperties.init("1.21.11");Map<String,Object> codecs=new HashMap<>();
  for(Object c:(Iterable<?>)ExportItemProperties.components){String name=ExportItemProperties.name.invoke(ExportItemProperties.components,c).toString();if(Set.of("minecraft:enchantable","minecraft:writable_book_content","minecraft:written_book_content").contains(name))codecs.put(name,Class.forName("kh").getMethod("f").invoke(c));}
  JsonArray rows=new JsonArray(),pairs=new JsonArray();List<Object> values=new ArrayList<>();
  for(JsonElement e:JsonParser.parseString(Files.readString(Path.of(args[0]))).getAsJsonArray()){
   JsonObject row=e.getAsJsonObject().deepCopy();String name=row.get("component").getAsString();Object codec=codecs.get(name),value;
   if(codec==null)throw new IllegalStateException("own unknown book component request");
   try{value=ExportComponentValueRules.decode(codec,HexFormat.of().parseHex(row.get("input_hex").getAsString()));}
   catch(InvocationTargetException|IllegalStateException error){Throwable t=ExportComponentNormalization.cause(error);if(t instanceof Error fatal)throw fatal;row.addProperty("accepted",false);row.addProperty("failure",t.getClass().getName()+": "+t.getMessage());rows.add(row);values.add(null);continue;}
   row.addProperty("accepted",true);row.add("fields",fields(name,value));row.addProperty("encoded_hex",HexFormat.of().formatHex(ExportEnchantmentConstructors.encode(codec,value)));rows.add(row);values.add(value);
  }
  for(int a=0;a<values.size();a++)for(int b=a;b<values.size();b++)if(values.get(a)!=null&&values.get(b)!=null){JsonObject p=new JsonObject();p.addProperty("a",a);p.addProperty("b",b);p.addProperty("equal",values.get(a).equals(values.get(b)));pairs.add(p);}
  JsonObject out=new JsonObject();out.add("cases",rows);out.add("pairs",pairs);out.addProperty("java_version",System.getProperty("java.version"));Files.writeString(Path.of(args[1]),ascii(new GsonBuilder().serializeNulls().create().toJson(out)));Class.forName("bas").getMethod("close").invoke(ExportItemComponents.resourceManager);System.out.println("original book cases="+rows.size()+" pairs="+pairs.size());
 }
}
