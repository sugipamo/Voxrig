// Original block-position serializer audit. Only this reflective harness is ours.
package voxrig.oracle;
import java.lang.reflect.*;
import java.security.MessageDigest;
import java.util.HexFormat;

public final class EntityPositionOracle {
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
    static Object decode(String[] args,Object serializer,Object buffer) throws Exception {
        Object decoded;
            if(args[6].equals("legacy")) {
                Method read=null;for(Method m:Class.forName(args[3]).getMethods())if(m.getName().equals(args[12])&&m.getParameterCount()==1&&m.getParameterTypes()[0]==Class.forName(args[5]))read=m;
                if(read==null)throw new IllegalArgumentException("missing original read method");
                decoded=invoke(read,serializer,buffer);
            }
            else {
                Object codec=invoke(method(Class.forName(args[3]),args[9],0),serializer);
                decoded=invoke(method(Class.forName(args[10]),args[12],1),codec,buffer);
            }
        return decoded;
    }
    public static void main(String[] args) throws Exception {
        java.io.PrintStream out=System.out;
        if(args.length>13)for(int i=13;i<15;i++) {
            if(args[i].equals("-"))continue;
            int dot=args[i].lastIndexOf('.');
            invoke(method(Class.forName(args[i].substring(0,dot)),args[i].substring(dot+1),0),null);
        }
        Class<?> positions=Class.forName(args[0]);Object serializer=field(args[1],args[2]);
        Class<?> serializerType=Class.forName(args[3]);
        int kind=(Integer)invoke(method(Class.forName(args[1]),args[4],1),null,serializer);
        Class<?> raw=Class.forName("io.netty.buffer.ByteBuf");
        Class<?> bufferType=Class.forName(args[5]);
        Class<?> unpooled=Class.forName("io.netty.buffer.Unpooled");
        Method make=unpooled.getMethod("buffer");
        boolean optional=args[2].equals(args[18]);
        int[][] points={{0,0,0},{-1,-1,-1},{33554431,2047,33554431},{-33554432,-2048,-33554432},{33554432,2048,-33554433},{Integer.MIN_VALUE,Integer.MAX_VALUE,Integer.MIN_VALUE},{-12000000,-64,17000000}};
        java.util.ArrayList<int[]> inputs=new java.util.ArrayList<>(java.util.Arrays.asList(points));
        java.util.Random random=new java.util.Random(20261012);
        for(int i=0;i<64;i++)inputs.add(new int[]{random.nextInt(),random.nextInt(),random.nextInt()});
        if(optional)inputs.add(0,null);
        Object accessor=field(args[15],args[16]);
        Field accessorId=accessor.getClass().getDeclaredField(args[17]);accessorId.setAccessible(true);
        out.println("KEY\t"+accessorId.get(accessor));
        out.println("SERIALIZER\t"+kind);
        for(int[] point:inputs) {
            Object position=point==null?null:positions.getConstructor(int.class,int.class,int.class).newInstance(point[0],point[1],point[2]);
            Object value=optional?java.util.Optional.ofNullable(position):position;
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
            
            Object decoded=decode(args,serializer,buffer);
            if((Integer)raw.getMethod("readableBytes").invoke(bytes)!=0)throw new AssertionError("original codec left bytes");
            Object actual=optional?((java.util.Optional<?>)decoded).orElse(null):decoded;
            String input=point==null?"null":point[0]+","+point[1]+","+point[2];
            String result="null";
            if(actual!=null) {
                int x=(Integer)invoke(method(positions,args[19],0),actual);
                int y=(Integer)invoke(method(positions,args[20],0),actual);
                int z=(Integer)invoke(method(positions,args[21],0),actual);
                if(x!=(point[0]<<6>>6)||y!=(point[1]<<20>>20)||z!=(point[2]<<6>>6))throw new AssertionError("original coordinate wrap");
                result=x+","+y+","+z;
            } else if(point!=null)throw new AssertionError("lost original position");
            out.println("POSITION\t"+input+"\t"+result+"\t"+size+"\t"+HexFormat.of().formatHex(MessageDigest.getInstance("SHA-256").digest(payload)));
            // Re-run the original codec at every truncated payload boundary.
            for(int cut=0;cut<size;cut++) {
                raw.getMethod("readerIndex",int.class).invoke(bytes,0);
                raw.getMethod("writerIndex",int.class).invoke(bytes,cut);
                try { decode(args,serializer,buffer); throw new AssertionError("original codec accepted truncated value"); }
                catch(InvocationTargetException expected) {
                    if(!(expected.getCause() instanceof IndexOutOfBoundsException))throw expected;
                }
            }
            raw.getMethod("readerIndex",int.class).invoke(bytes,0);
            raw.getMethod("writerIndex",int.class).invoke(bytes,size);
            if(optional&&point!=null) {
                raw.getMethod("setByte",int.class,int.class).invoke(bytes,0,255);
                if(!decode(args,serializer,buffer).equals(decoded))throw new AssertionError("original nonzero presence marker");
            }
            raw.getMethod("release").invoke(bytes);
        }
    }
}
