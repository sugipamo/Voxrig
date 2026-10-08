// Own observer of unchanged original boss-event readers, writers and enums.
import com.google.gson.*;
import io.netty.buffer.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
public final class ExportBossBars {
    static boolean old; static Object registries;
    static Object buffer(byte[] raw) throws Exception {
        ByteBuf b=raw==null?Unpooled.buffer():Unpooled.wrappedBuffer(raw);
        return old?Class.forName("mg").getConstructor(ByteBuf.class).newInstance(b):Class.forName("xq").getConstructor(ByteBuf.class,Class.forName("jr")).newInstance(b,registries);
    }
    static Object field(Object o,String name)throws Exception {Field f=o.getClass().getDeclaredField(name);f.setAccessible(true);return f.get(o);}
    static byte[] bytes(Object o){ByteBuf b=(ByteBuf)o;byte[] out=new byte[b.readableBytes()];b.getBytes(b.readerIndex(),out);return out;}
    static void varint(ByteBuf b,int value){do{int next=value&127;value>>>=7;b.writeByte(next|(value!=0?128:0));}while(value!=0);}
    static byte[] request(int op,int color,int overlay,int flags,float progress,String title){
        ByteBuf b=Unpooled.buffer();try{
            for(int n=0;n<16;n++)b.writeByte(n+1);varint(b,op);
            if(op==0||op==3){byte[] text=(old?("{\"text\":\""+title+"\"}"):title).getBytes(java.nio.charset.StandardCharsets.UTF_8);if(old)varint(b,text.length);else{b.writeByte(8);b.writeShort(text.length);}b.writeBytes(text);}
            if(op==0||op==2)b.writeFloat(progress);
            if(op==0||op==4){varint(b,color);varint(b,overlay);}
            if(op==0||op==5)b.writeByte(flags);
            return bytes(b);
        }finally{b.release();}
    }
    static void facts(JsonObject row,int op,Object title,Float progress,Object color,Object overlay,boolean[] flags)throws Exception{
        row.addProperty("operation",op);
        if(title!=null)row.addProperty("native_title_text",(String)Class.forName(old?"mr":"yh").getMethod("getString").invoke(title));
        if(progress!=null)row.addProperty("progress",progress);
        if(color!=null)row.addProperty("color",((Enum<?>)color).ordinal());
        if(overlay!=null)row.addProperty("overlay",((Enum<?>)overlay).ordinal());
        if(flags!=null){JsonArray a=new JsonArray();for(boolean value:flags)a.add(value);row.add("flags",a);}
    }
    static JsonObject observe(byte[] input)throws Exception{
        Object read=buffer(input),write=buffer(null);Class<?> packet=Class.forName(old?"ny":"adk");JsonObject row=new JsonObject();Object value;
        try{
            if(old){value=packet.getConstructor().newInstance();packet.getMethod("a",Class.forName("mg")).invoke(value,read);packet.getMethod("b",Class.forName("mg")).invoke(value,write);
                int op=((Enum<?>)field(value,"b")).ordinal();row.addProperty("uuid",field(value,"a").toString());
                facts(row,op,op==0||op==3?field(value,"c"):null,op==0||op==2?(Float)field(value,"d"):null,op==0||op==4?field(value,"e"):null,op==0||op==4?field(value,"f"):null,op==0||op==5?new boolean[]{(boolean)field(value,"g"),(boolean)field(value,"h"),(boolean)field(value,"i")}:null);
            }else{
                Object codec=packet.getField("a").get(null);Class<?> stream=Class.forName("aao");value=stream.getMethod("decode",Object.class).invoke(codec,read);stream.getMethod("encode",Object.class,Object.class).invoke(codec,write,value);
                Class<?> handler=Class.forName("adk$b");Object observer=Proxy.newProxyInstance(handler.getClassLoader(),new Class<?>[]{handler},(p,m,a)->{
                    int op=a.length==8?0:a.length==1?1:a.length==4?5:a.length==3?4:a[1] instanceof Float?2:3;
                    row.addProperty("uuid",a[0].toString());facts(row,op,op==0||op==3?a[1]:null,op==0?(Float)a[2]:op==2?(Float)a[1]:null,op==0?a[3]:op==4?a[1]:null,op==0?a[4]:op==4?a[2]:null,op==0?new boolean[]{(boolean)a[5],(boolean)a[6],(boolean)a[7]}:op==5?new boolean[]{(boolean)a[1],(boolean)a[2],(boolean)a[3]}:null);return null;});
                packet.getMethod("a",handler).invoke(value,observer);
            }
            if(((ByteBuf)read).isReadable())throw new IllegalStateException("native reader left fields");
            row.addProperty("payload_hex",HexFormat.of().formatHex(bytes(write)));return row;
        }finally{((ByteBuf)read).release();((ByteBuf)write).release();}
    }
    public static void main(String[] args)throws Exception{
        old=args[0].equals("1.16.1");if(!old){Class.forName("w").getMethod("a").invoke(null);Class.forName("amv").getMethod("a").invoke(null);registries=Class.forName("jr").getMethod("a",Class.forName("jq")).invoke(null,Class.forName("mi").getField("aR").get(null));}
        JsonObject out=new JsonObject();out.addProperty("version",args[0]);JsonArray rows=new JsonArray();
        for(int op=0;op<6;op++)rows.add(observe(request(op,2,4,5,.5f,op==3?"Updated":"Boss")));
        for(int color=0;color<7;color++)rows.add(observe(request(4,color,0,0,.5f,"Boss")));
        for(int overlay=0;overlay<5;overlay++)rows.add(observe(request(4,0,overlay,0,.5f,"Boss")));
        for(int flags=0;flags<8;flags++)rows.add(observe(request(5,0,0,flags,.5f,"Boss")));
        for(float progress:new float[]{-.25f,0,.25f,1,1.5f})rows.add(observe(request(2,0,0,0,progress,"Boss")));
        out.add("packets",rows);Files.writeString(Path.of(args[1]),new GsonBuilder().setPrettyPrinting().create().toJson(out)+"\n");
    }
}
