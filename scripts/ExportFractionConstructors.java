// Own observer of the original bundled Fraction arithmetic used by bundle constructors.
import com.google.gson.*;
import java.nio.file.*;
import java.io.*;
import java.util.*;
import org.apache.commons.lang3.math.Fraction;
public class ExportFractionConstructors {
 public static void main(String[] args)throws Exception {
  try(PrintWriter log=new PrintWriter(Files.newBufferedWriter(Path.of(args[1]).getParent().resolve("original-fraction-bytecode.log")))){int status=java.util.spi.ToolProvider.findFirst("javap").orElseThrow().run(log,log,"-classpath",System.getProperty("java.class.path"),"-p","-c","org.apache.commons.lang3.math.Fraction");if(status!=0)throw new IllegalStateException("original Fraction inspection failed");}
  JsonArray rows=new JsonArray(),pairs=new JsonArray();List<Fraction> values=new ArrayList<>();
  for(JsonElement e:JsonParser.parseString(Files.readString(Path.of(args[0]))).getAsJsonArray()){
   JsonObject row=e.getAsJsonObject().deepCopy();JsonArray a=row.getAsJsonArray("arguments");Fraction value;
   try{Fraction left=Fraction.getFraction(a.get(0).getAsInt(),a.get(1).getAsInt());String op=row.get("operation").getAsString();value=switch(op){case "from" -> left;case "add" -> left.add(Fraction.getFraction(a.get(2).getAsInt(),a.get(3).getAsInt()));case "multiply" -> left.multiplyBy(Fraction.getFraction(a.get(2).getAsInt(),a.get(3).getAsInt()));default -> throw new IllegalStateException("own unknown Fraction request");};}
   catch(ArithmeticException error){row.addProperty("accepted",false);row.addProperty("failure",error.getClass().getName()+": "+error.getMessage());rows.add(row);values.add(null);continue;}
   row.addProperty("accepted",true);row.addProperty("numerator",value.getNumerator());row.addProperty("denominator",value.getDenominator());rows.add(row);values.add(value);
  }
  for(int a=0;a<values.size();a++)for(int b=a;b<values.size();b++)if(values.get(a)!=null&&values.get(b)!=null){JsonObject p=new JsonObject();p.addProperty("a",a);p.addProperty("b",b);p.addProperty("equal",values.get(a).equals(values.get(b)));pairs.add(p);}
  JsonObject out=new JsonObject();out.add("cases",rows);out.add("pairs",pairs);out.addProperty("java_version",System.getProperty("java.version"));Files.writeString(Path.of(args[1]),new GsonBuilder().serializeNulls().create().toJson(out));System.out.println("original Fraction cases="+rows.size()+" pairs="+pairs.size());
 }
}
