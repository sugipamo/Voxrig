// Observe unchanged original mounted-input serializers and decoded fields.
// No game handler, vehicle physics or World is replaced.
import io.netty.buffer.ByteBuf;
import io.netty.buffer.Unpooled;
import com.google.gson.*;
import java.lang.reflect.*;
import java.nio.*;
import java.nio.file.*;
public final class ExportVehicleControl {
    static Object buffer(boolean old, byte[] input) throws Exception {
        ByteBuf bytes=input==null?Unpooled.buffer():Unpooled.wrappedBuffer(input);
        return Class.forName(old?"mg":"wx").getConstructor(ByteBuf.class).newInstance(bytes);
    }
    static String hex(Object buffer) {
        ByteBuf bytes=(ByteBuf)buffer;byte[] out=new byte[bytes.readableBytes()];bytes.getBytes(bytes.readerIndex(),out);
        return java.util.HexFormat.of().formatHex(out);
    }
    static Object field(Object target,String name)throws Exception {
        Field f=target.getClass().getDeclaredField(name);f.setAccessible(true);return f.get(target);
    }
    public static void main(String[] args)throws Exception {
        boolean old=args[0].equals("1.16.1");Class<?> buf=Class.forName(old?"mg":"wx");JsonArray inputs=new JsonArray();
        for(int forward=-1;forward<=1;forward++)for(int strafe=-1;strafe<=1;strafe++)for(boolean jump:new boolean[]{false,true}) {
            Object packet;
            if(old) {
                // The convenience constructor is stripped from the server artifact.
                // Populate only by its unchanged native reader, then inspect fields.
                packet=Class.forName("sa").getConstructor().newInstance();
                byte[] raw=ByteBuffer.allocate(9).putFloat(strafe).putFloat(forward).put((byte)(jump?1:0)).array();
                Object reader=buffer(true,raw);
                try {Class.forName("sa").getMethod("a",buf).invoke(packet,reader);
                    if(((ByteBuf)reader).readableBytes()!=0 || !field(packet,"a").equals((float)strafe) || !field(packet,"b").equals((float)forward)
                        || !field(packet,"c").equals(jump) || !field(packet,"d").equals(false))throw new IllegalStateException("native legacy fields differ");
                }finally {((ByteBuf)reader).release();}
            }else {
                Object input=Class.forName("ddk").getConstructor(boolean.class,boolean.class,boolean.class,boolean.class,boolean.class,boolean.class,boolean.class)
                    .newInstance(forward==1,forward==-1,strafe==1,strafe==-1,jump,false,false);
                packet=Class.forName("ajk").getConstructor(Class.forName("ddk")).newInstance(input);
            }
            Object encoded=buffer(old,null);
            try {
                if(old)Class.forName("sa").getMethod("b",buf).invoke(packet,encoded);
                else Class.forName("aao").getMethod("encode",Object.class,Object.class).invoke(Class.forName("ajk").getField("a").get(null),encoded,packet);
                JsonObject row=new JsonObject();row.addProperty("forward",forward);row.addProperty("strafe",strafe);row.addProperty("jump",jump);row.addProperty("payload_hex",hex(encoded));inputs.add(row);
                if(!old) {
                    Object decoded=Class.forName("aao").getMethod("decode",Object.class).invoke(Class.forName("ajk").getField("a").get(null),encoded);
                    Object input=field(decoded,"b");
                    boolean[] expected={forward==1,forward==-1,strafe==1,strafe==-1,jump,false,false};
                    for(int i=0;i<expected.length;i++)if(!field(input,String.valueOf((char)('c'+i))).equals(expected[i]))throw new IllegalStateException("native modern decoded field differs");
                    if(((ByteBuf)encoded).readableBytes()!=0)throw new IllegalStateException("native modern reader left bytes");
                }
            }finally {((ByteBuf)encoded).release();}
        }
        JsonObject out=new JsonObject();out.addProperty("version",args[0]);out.add("inputs",inputs);
        Files.writeString(Path.of(args[1]),new GsonBuilder().setPrettyPrinting().create().toJson(out)+"\n");
    }
}
