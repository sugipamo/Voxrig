// Own reflective observer of original title/tab/border packet codecs and fields.
import com.google.gson.*;
import io.netty.buffer.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
public final class ExportDisplayPackets {
    static boolean old;static Object registries;
    static Object buffer(byte[] raw)throws Exception{ByteBuf b=raw==null?Unpooled.buffer():Unpooled.wrappedBuffer(raw);return old?Class.forName("mg").getConstructor(ByteBuf.class).newInstance(b):Class.forName("xq").getConstructor(ByteBuf.class,Class.forName("jr")).newInstance(b,registries);}
    static Object field(Object o,String name)throws Exception{Field f=o.getClass().getDeclaredField(name);f.setAccessible(true);return f.get(o);}
    static byte[] bytes(Object o){ByteBuf b=(ByteBuf)o;byte[] raw=new byte[b.readableBytes()];b.getBytes(b.readerIndex(),raw);return raw;}
    static void variable(ByteBuf b,long value){do{int next=(int)(value&127);value>>>=7;b.writeByte(next|(value!=0?128:0));}while(value!=0);}
    static void text(ByteBuf b,String value){byte[] raw=(old?("{\"text\":\""+value+"\"}"):value).getBytes(java.nio.charset.StandardCharsets.UTF_8);if(old)variable(b,raw.length);else{b.writeByte(8);b.writeShort(raw.length);}b.writeBytes(raw);}
    static String plain(Object value)throws Exception{return(String)Class.forName(old?"mr":"yh").getMethod("getString").invoke(value);}
    static byte[] request(String group,int op,long duration)throws Exception{
        ByteBuf b=Unpooled.buffer();try{
            if(old&&!group.equals("tab"))variable(b,op);
            if(group.equals("title")){if(op<3)text(b,new String[]{"Title","Subtitle","Overlay"}[op]);else if(op==3){b.writeInt(3);b.writeInt(-1);b.writeInt(9);}else if(!old)b.writeBoolean(op==5);}
            else if(group.equals("tab")){text(b,"Header雪");text(b,"Footer");}
            else if(op==0)b.writeDouble(128.5);
            else if(op==1){b.writeDouble(100);b.writeDouble(200);variable(b,duration);}
            else if(op==2){b.writeDouble(10.25);b.writeDouble(-20.5);}
            else if(op==3){b.writeDouble(10.25);b.writeDouble(-20.5);b.writeDouble(100);b.writeDouble(200);variable(b,duration);variable(b,60000000);variable(b,3);variable(b,17);}
            else variable(b,op==4?17:3);
            return bytes(b);
        }finally{b.release();}
    }
    static JsonObject observe(String group,int op,long duration)throws Exception{
        String name;int id;
        if(old){name=group.equals("title")?"qk":group.equals("tab")?"qo":"ps";id=group.equals("title")?0x4f:group.equals("tab")?0x53:0x3d;}
        else if(group.equals("title")){name=new String[]{"ahe","ahc","agd","ahf","adr","adr"}[op];id=new int[]{0x70,0x6e,0x55,0x71,0x0e,0x0e}[op];}
        else if(group.equals("tab")){name="ahl";id=0x78;}
        else{name=new String[]{"agg","agf","age","aep","agh","agi"}[op];id=new int[]{0x58,0x57,0x56,0x2a,0x59,0x5a}[op];}
        Class<?> type=Class.forName(name);Object read=buffer(request(group,op,duration)),write=buffer(null),packet;
        try{
            if(old){packet=type.getConstructor().newInstance();type.getMethod("a",Class.forName("mg")).invoke(packet,read);type.getMethod("b",Class.forName("mg")).invoke(packet,write);}
            else{Object codec=type.getField("a").get(null);Class<?> stream=Class.forName("aao");packet=stream.getMethod("decode",Object.class).invoke(codec,read);stream.getMethod("encode",Object.class,Object.class).invoke(codec,write,packet);}
            if(((ByteBuf)read).isReadable())throw new IllegalStateException("original display reader left fields");
            JsonObject row=new JsonObject();row.addProperty("group",group);row.addProperty("operation",op);row.addProperty("packet_id",id);row.addProperty("payload_hex",HexFormat.of().formatHex(bytes(write)));
            if(group.equals("title")){
                if(old&&((Enum<?>)field(packet,"a")).ordinal()!=op)throw new IllegalStateException("native title operation differs");
                if(op<3)row.addProperty("text",plain(field(packet,"b")));
                else if(op==3){row.addProperty("fade_in",((Number)field(packet,old?"c":"b")).intValue());row.addProperty("stay",((Number)field(packet,old?"d":"c")).intValue());row.addProperty("fade_out",((Number)field(packet,old?"e":"d")).intValue());}
                else row.addProperty("reset_times",old?op==5:(boolean)field(packet,"b"));
            }else if(group.equals("tab")){row.addProperty("header",plain(field(packet,old?"a":"b")));row.addProperty("footer",plain(field(packet,old?"b":"c")));}
            else{
                if(old&&((Enum<?>)field(packet,"a")).ordinal()!=op)throw new IllegalStateException("native border operation differs");
                if(op==0)row.addProperty("diameter",((Number)field(packet,old?"e":"b")).doubleValue());
                if(op==1||op==3){row.addProperty("from_diameter",((Number)field(packet,old?"f":op==1?"b":"d")).doubleValue());row.addProperty("to_diameter",((Number)field(packet,old?"e":op==1?"c":"e")).doubleValue());JsonObject time=new JsonObject();time.addProperty("unit",old?"milliseconds":"ticks");time.addProperty("value",((Number)field(packet,old?"g":op==1?"d":"f")).longValue());row.add("duration",time);}
                if(op==2||op==3){row.addProperty("center_x",((Number)field(packet,old?"c":"b")).doubleValue());row.addProperty("center_z",((Number)field(packet,old?"d":"c")).doubleValue());}
                if(op==3){row.addProperty("absolute_max_size",((Number)field(packet,old?"b":"g")).intValue());row.addProperty("warning_delay",((Number)field(packet,old?"h":"i")).intValue());row.addProperty("warning_distance",((Number)field(packet,old?"i":"h")).intValue());}
                if(op==4)row.addProperty("warning_delay",((Number)field(packet,old?"h":"b")).intValue());
                if(op==5)row.addProperty("warning_distance",((Number)field(packet,old?"i":"b")).intValue());
            }
            return row;
        }finally{((ByteBuf)read).release();((ByteBuf)write).release();}
    }
    public static void main(String[]args)throws Exception{
        old=args[0].equals("1.16.1");if(!old){Class.forName("w").getMethod("a").invoke(null);Class.forName("amv").getMethod("a").invoke(null);registries=Class.forName("jr").getMethod("a",Class.forName("jq")).invoke(null,Class.forName("mi").getField("aR").get(null));}
        JsonObject out=new JsonObject();out.addProperty("version",args[0]);JsonArray rows=new JsonArray();
        for(int op=0;op<6;op++)rows.add(observe("title",op,0));rows.add(observe("tab",0,0));
        for(int op=0;op<6;op++)rows.add(observe("border",op,10000));
        for(long duration:new long[]{0,1L<<45,Long.MAX_VALUE,-1}){rows.add(observe("border",1,duration));rows.add(observe("border",3,duration));}
        out.add("packets",rows);Files.writeString(Path.of(args[1]),new GsonBuilder().setPrettyPrinting().create().toJson(out)+"\n");
    }
}
