// Own reflective observer: unchanged original packet readers/writers only.
// No native tick, riding algorithm or handler is copied or replaced.
import io.netty.buffer.ByteBuf;
import io.netty.buffer.Unpooled;
import com.google.gson.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
public final class ExportVehicleInput {
    static Object buffer(boolean old, byte[] input) throws Exception {
        ByteBuf bytes=input==null?Unpooled.buffer():Unpooled.wrappedBuffer(input);
        return Class.forName(old?"mg":"wx").getConstructor(ByteBuf.class).newInstance(bytes);
    }
    static String hex(Object buffer) {
        ByteBuf bytes=(ByteBuf)buffer;byte[] out=new byte[bytes.readableBytes()];bytes.getBytes(bytes.readerIndex(),out);
        return HexFormat.of().formatHex(out);
    }
    public static void main(String[] args) throws Exception {
        boolean old=args[0].equals("1.16.1");Class<?> buf=Class.forName(old?"mg":"wx");
        JsonObject out=new JsonObject();out.addProperty("version",args[0]);JsonArray inputs=new JsonArray();
        for(boolean shift:List.of(true,false)) {
            Object packet;
            if(old) {
                packet=Class.forName("sa").getConstructor().newInstance();
                byte[] input=new byte[9];input[8]=(byte)(shift?2:0);Object original=buffer(true,input);
                try {Class.forName("sa").getMethod("a",buf).invoke(packet,original);}
                finally {((ByteBuf)original).release();}
            }
            else {
                Object input=Class.forName("ddk").getConstructor(boolean.class,boolean.class,boolean.class,boolean.class,boolean.class,boolean.class,boolean.class).newInstance(false,false,false,false,false,shift,false);
                packet=Class.forName("ajk").getConstructor(Class.forName("ddk")).newInstance(input);
            }
            Object encoded=buffer(old,null);
            try {
                if(old)Class.forName("sa").getMethod("b",buf).invoke(packet,encoded);
                else Class.forName("aao").getMethod("encode",Object.class,Object.class).invoke(Class.forName("ajk").getField("a").get(null),encoded,packet);
                JsonObject row=new JsonObject();row.addProperty("shift",shift);row.addProperty("payload_hex",hex(encoded));inputs.add(row);
            } finally {((ByteBuf)encoded).release();}
        }
        JsonArray passengers=new JsonArray();
        for(byte[] payload:List.of(new byte[]{10,1,42},new byte[]{10,0},new byte[]{(byte)128,1,2,(byte)172,2,42})) {
            Object encoded=buffer(old,null),decoded=buffer(old,payload);
            try {
                Object packet;
                if(old) {packet=Class.forName("qg").getConstructor().newInstance();Class.forName("qg").getMethod("a",buf).invoke(packet,decoded);Class.forName("qg").getMethod("b",buf).invoke(packet,encoded);}
                else {Constructor<?> c=Class.forName("agx").getDeclaredConstructor(buf);c.setAccessible(true);packet=c.newInstance(decoded);Method write=Class.forName("agx").getDeclaredMethod("a",buf);write.setAccessible(true);write.invoke(packet,encoded);}
                if(((ByteBuf)decoded).readableBytes()!=0)throw new IllegalStateException("native passenger reader left bytes");
                String roundtrip=hex(encoded);if(!roundtrip.equals(HexFormat.of().formatHex(payload)))throw new IllegalStateException("native passenger roundtrip changed");
                JsonObject row=new JsonObject();row.addProperty("payload_hex",roundtrip);
                Field vehicle=packet.getClass().getDeclaredField(old?"a":"b");vehicle.setAccessible(true);
                row.addProperty("vehicle",vehicle.getInt(packet));
                Field ids=packet.getClass().getDeclaredField(old?"b":"c");ids.setAccessible(true);
                JsonArray members=new JsonArray();for(int id:(int[])ids.get(packet))members.add(id);row.add("passengers",members);passengers.add(row);
            } finally {((ByteBuf)encoded).release();((ByteBuf)decoded).release();}
        }
        out.add("inputs",inputs);out.add("passengers",passengers);
        Files.writeString(Path.of(args[1]),new GsonBuilder().setPrettyPrinting().create().toJson(out)+"\n");
    }
}
