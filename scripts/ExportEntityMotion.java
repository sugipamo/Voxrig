// Original position codecs only; no World, interpolation or game handler replacement.
import com.google.gson.*;
import io.netty.buffer.*;
import java.nio.file.*;
import java.lang.reflect.*;
public final class ExportEntityMotion {
    public static void main(String[] args) throws Exception {
        boolean old=args[0].equals("1.16.1");
        Class<?> vec=Class.forName(old?"dem":"ftm"), codec=Class.forName(old?"pa":"akg");
        double[][] bases={{0,64,0},{0.50001,65.0625,2.50009},{-0.0001220703125,-65.12345,-2.5001},{0.0001220703125,0.000244140625,-0.000244140625},{-1.5,0,1.5},{123456.789,256.1,-123456.789},{1.0/3,2.0/3,-1.0/3},{-0.00001,0.00001,0.99999}};
        short[][] deltas={{0,0,0},{1,0,0},{0,-1,0},{0,0,1},{-1,1,-1},{32767,-32768,0},{8,16,-32},{-128,0,128}};
        JsonArray rows=new JsonArray();
        for(double[] base:bases)for(short[] delta:deltas) {
            Object result;
            if(old) {
                long[] encoded=new long[3];
                for(int i=0;i<3;i++)encoded[i]=(long)codec.getMethod("a",double.class).invoke(null,base[i])+delta[i];
                result=codec.getMethod("a",long.class,long.class,long.class).invoke(null,encoded[0],encoded[1],encoded[2]);
            }else {
                Object instance=codec.getConstructor().newInstance();
                codec.getMethod("e",vec).invoke(instance,vec.getConstructor(double.class,double.class,double.class).newInstance(base[0],base[1],base[2]));
                result=codec.getMethod("a",long.class,long.class,long.class).invoke(instance,(long)delta[0],(long)delta[1],(long)delta[2]);
            }
            JsonObject row=new JsonObject();row.add("base",new Gson().toJsonTree(base));row.add("delta",new Gson().toJsonTree(delta));
            double[] position=new double[3];for(int i=0;i<3;i++)position[i]=vec.getField(String.valueOf((char)((old?'b':'g')+i))).getDouble(result);
            row.add("position",new Gson().toJsonTree(position));rows.add(row);
        }
        JsonObject out=new JsonObject();out.addProperty("version",args[0]);out.add("relative_positions",rows);
        if(old) {
            Object packet=codec.getConstructor().newInstance();Class<?> buf=Class.forName("mg");
            ByteBuf bytes=Unpooled.wrappedBuffer(new byte[]{42});Object input=buf.getConstructor(ByteBuf.class).newInstance(bytes);
            codec.getMethod("a",buf).invoke(packet,input);
            if(bytes.readableBytes()!=0)throw new IllegalStateException("legacy base packet trailing bytes");
            ByteBuf encoded=Unpooled.buffer();Object output=buf.getConstructor(ByteBuf.class).newInstance(encoded);codec.getMethod("b",buf).invoke(packet,output);
            byte[] raw=new byte[encoded.readableBytes()];encoded.readBytes(raw);out.addProperty("legacy_base_payload_hex",java.util.HexFormat.of().formatHex(raw));
            bytes.release();encoded.release();
        }
        Files.writeString(Path.of(args[1]),new GsonBuilder().setPrettyPrinting().create().toJson(out)+"\n");
    }
}
