// Original pose enum and serializer audit. Only this reflective harness is ours.
package voxrig.oracle;
import java.lang.reflect.*;
import java.security.MessageDigest;
import java.util.HexFormat;

public final class EntityPoseOracle {
    static Object invoke(Method method,Object target,Object... args) throws Exception {
        method.setAccessible(true);return method.invoke(target,args);
    }
    static Method method(Class<?> type,String name,int count) {
        for(Method m:type.getMethods())if(m.getName().equals(name)&&m.getParameterCount()==count)return m;
        throw new IllegalArgumentException(type+" method "+name);
    }
    static Object field(String owner,String name) throws Exception {
        Field f=Class.forName(owner).getDeclaredField(name);f.setAccessible(true);return f.get(null);
    }
    public static void main(String[] args) throws Exception {
        java.io.PrintStream out=System.out;
        if(args.length>13)for(int i=13;i<15;i++) {
            if(args[i].equals("-"))continue;
            int dot=args[i].lastIndexOf('.');
            invoke(method(Class.forName(args[i].substring(0,dot)),args[i].substring(dot+1),0),null);
        }
        Class<?> poses=Class.forName(args[0]);Object serializer=field(args[1],args[2]);
        Class<?> serializerType=Class.forName(args[3]);
        int kind=(Integer)invoke(method(Class.forName(args[1]),args[4],1),null,serializer);
        Class<?> raw=Class.forName("io.netty.buffer.ByteBuf");
        Class<?> bufferType=Class.forName(args[5]);
        Class<?> unpooled=Class.forName("io.netty.buffer.Unpooled");
        Method make=unpooled.getMethod("buffer");
        Object[] values=poses.getEnumConstants();
        Object accessor=field(args[15],args[16]);
        Field accessorId=accessor.getClass().getDeclaredField(args[17]);accessorId.setAccessible(true);
        out.println("KEY\t"+accessorId.get(accessor));
        out.println("SERIALIZER\t"+kind);
        for(Object value:values) {
            Object bytes=make.invoke(null);Object buffer;
            if(args[6].equals("legacy"))buffer=bufferType.getConstructor(raw).newInstance(bytes);
            else buffer=bufferType.getConstructors()[0].newInstance(bytes,field(args[7],args[8]));
            if(args[6].equals("legacy"))invoke(method(serializerType,args[9],2),serializer,buffer,value);
            else {
                Object codec=invoke(method(serializerType,args[9],0),serializer);
                invoke(method(Class.forName(args[10]),args[11],2),codec,buffer,value);
            }
            int size=(Integer)raw.getMethod("readableBytes").invoke(bytes);
            byte[] payload=new byte[size];raw.getMethod("getBytes",int.class,byte[].class).invoke(bytes,0,payload);
            int encoded=0,shift=0;for(byte b:payload){encoded|=(b&127)<<shift;shift+=7;if((b&128)==0)break;}
            Object decoded;
            if(args[6].equals("legacy")) {
                Method read=null;for(Method m:serializerType.getMethods())if(m.getName().equals(args[12])&&m.getParameterCount()==1&&m.getParameterTypes()[0]==bufferType)read=m;
                if(read==null)throw new IllegalArgumentException("missing original read method");
                decoded=invoke(read,serializer,buffer);
            }
            else {
                Object codec=invoke(method(serializerType,args[9],0),serializer);
                decoded=invoke(method(Class.forName(args[10]),args[12],1),codec,buffer);
            }
            if(decoded!=value||(Integer)raw.getMethod("readableBytes").invoke(bytes)!=0)throw new AssertionError("pose round trip");
            out.println("POSE\t"+((Enum<?>)value).ordinal()+"\t"+encoded+"\t"+((Enum<?>)value).name()+"\t"+size+"\t"+HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(payload)));
            raw.getMethod("release").invoke(bytes);
        }
    }
}
