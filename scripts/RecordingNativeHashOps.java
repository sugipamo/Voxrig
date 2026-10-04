// Own observation wrapper: every hash operation/builder delegates to unchanged HashOps.
// Records only typed inputs and returned factual hashes; no game method is replaced.
import com.google.gson.*;
import com.google.common.hash.HashCode;
import com.mojang.serialization.*;
import com.mojang.datafixers.util.Pair;
import java.lang.reflect.*;
import java.nio.ByteBuffer;
import java.util.*;
import java.util.stream.*;

public final class RecordingNativeHashOps implements InvocationHandler {
    public static final class Graph {
        final JsonArray nodes = new JsonArray();
        final Map<String,Integer> interned = new HashMap<>();
        int intern(JsonObject node) {
            String key=node.toString();Integer id=interned.get(key);
            if(id==null) {id=nodes.size();interned.put(key,id);nodes.add(node);}
            return id;
        }
    }
    final Graph graph;
    final DynamicOps<HashCode> original;
    final IdentityHashMap<HashCode,Integer> ledger = new IdentityHashMap<>();
    final DynamicOps<HashCode> ops;
    @SuppressWarnings("unchecked")
    public RecordingNativeHashOps(Graph graph) throws Exception {
        this.graph=graph;original=(DynamicOps<HashCode>)Class.forName("bfw").getField("c").get(null);
        ops=(DynamicOps<HashCode>)Proxy.newProxyInstance(DynamicOps.class.getClassLoader(),new Class<?>[]{DynamicOps.class},this);
    }
    public DynamicOps<HashCode> ops() {return ops;}
    int node(HashCode value) {
        Integer index=ledger.get(value);
        if(index==null)throw new IllegalStateException("own recorder missing native result identity: "+value);
        return index;
    }
    int record(HashCode value,JsonObject description) {
        int index=graph.intern(description);Integer previous=ledger.put(value,index);
        if(previous!=null && previous!=index)throw new IllegalStateException("own recorder conflicting native identity");
        return index;
    }
    static JsonObject kind(String kind) {JsonObject node=new JsonObject();node.addProperty("kind",kind);return node;}
    static JsonObject string(String value) {
        JsonObject node=kind("string");JsonArray units=new JsonArray();for(char c:value.toCharArray())units.add((int)c);node.add("units",units);return node;
    }
    JsonObject scalar(String method,Object argument) {
        JsonObject node;
        switch(method) {
            case "createNumeric":
                if(argument instanceof Byte)return scalar("createByte",argument);
                if(argument instanceof Short)return scalar("createShort",argument);
                if(argument instanceof Integer)return scalar("createInt",argument);
                if(argument instanceof Long)return scalar("createLong",argument);
                if(argument instanceof Float)return scalar("createFloat",argument);
                return scalar("createDouble",argument);
            case "createByte":node=kind("byte");node.addProperty("value",((Number)argument).byteValue());return node;
            case "createShort":node=kind("short");node.addProperty("value",((Number)argument).shortValue());return node;
            case "createInt":node=kind("int");node.addProperty("value",((Number)argument).intValue());return node;
            case "createLong":node=kind("long");node.addProperty("value",((Number)argument).longValue());return node;
            case "createFloat":node=kind("float");node.addProperty("bits",Integer.toUnsignedLong(Float.floatToRawIntBits(((Number)argument).floatValue())));return node;
            case "createDouble":node=kind("double");node.addProperty("bits",Long.toUnsignedString(Double.doubleToRawLongBits(((Number)argument).doubleValue())));return node;
            case "createBoolean":node=kind("boolean");node.addProperty("value",(Boolean)argument);return node;
            case "createString":return string((String)argument);
            default:return null;
        }
    }
    Object value(Object value) {
        if(value instanceof DataResult<?> result)return result.result().orElse(null);
        return value;
    }
    int child(Object input) {
        Object child=value(input);
        if(child instanceof String text)return graph.intern(string(text));
        if(child instanceof HashCode code)return node(code);
        throw new IllegalStateException("own recorder unsupported builder input: "+child);
    }
    JsonObject list(List<Integer> children) {
        JsonObject node=kind("list");JsonArray values=new JsonArray();for(int child:children)values.add(child);node.add("values",values);return node;
    }
    JsonObject map(List<int[]> children) {
        // Canonical description ordering only; original HashOps performs its own hash sorting.
        children=new ArrayList<>(children);children.sort(Comparator.comparingInt((int[] e)->e[0]).thenComparingInt(e->e[1]));
        JsonObject node=kind("map");JsonArray entries=new JsonArray();for(int[] entry:children) {JsonArray pair=new JsonArray();pair.add(entry[0]);pair.add(entry[1]);entries.add(pair);}node.add("entries",entries);return node;
    }
    Object invokeOriginal(Object target,Method method,Object[] args) throws Throwable {
        try {return method.invoke(target,args);}catch(InvocationTargetException e) {throw e.getCause();}
    }
    Object builder(Object delegate,Class<?> iface) {
        boolean isMap=iface==RecordBuilder.class;List<int[]> entries=new ArrayList<>();List<Integer> children=new ArrayList<>();
        InvocationHandler handler=(proxy,method,arguments)-> {
            Object[] args=arguments==null?new Object[0]:arguments;
            if(method.getDeclaringClass()==Object.class) {
                return switch(method.getName()) {case "equals"->proxy==args[0];case "hashCode"->System.identityHashCode(proxy);case "toString"->"OwnObservedNativeBuilder";default->throw new IllegalStateException();};
            }
            Object result=invokeOriginal(delegate,method,args);
            if(method.getName().equals("ops"))return ops;
            if(method.getName().equals("add")) {
                if(isMap) {if(value(args[0])!=null && value(args[1])!=null)entries.add(new int[]{child(args[0]),child(args[1])});}
                else if(value(args[0])!=null)children.add(child(args[0]));
            }
            if(method.getName().equals("build")) {
                Object built=value(result);
                if(built instanceof HashCode code)record(code,isMap?map(entries):list(children));
                entries.clear();children.clear();
            }
            return iface.isInstance(result)?proxy:result;
        };
        return Proxy.newProxyInstance(iface.getClassLoader(),new Class<?>[]{iface},handler);
    }
    @Override public Object invoke(Object proxy,Method method,Object[] arguments) throws Throwable {
        Object[] args=arguments==null?new Object[0]:arguments;
        String name=method.getName();
        if(method.getDeclaringClass()==Object.class) {
            return switch(name) {case "equals"->proxy==args[0];case "hashCode"->System.identityHashCode(proxy);case "toString"->"OwnObservedNativeHashOps";default->throw new IllegalStateException();};
        }
        JsonObject description=args.length==1?scalar(name,args[0]):null;
        if(name.equals("empty"))description=kind("empty");
        if(name.equals("emptyMap"))description=map(List.of());
        if(name.equals("emptyList"))description=list(List.of());
        if(name.equals("createList")) {
            @SuppressWarnings("unchecked") List<HashCode> values=((Stream<HashCode>)args[0]).toList();
            description=list(values.stream().map(this::node).toList());args[0]=values.stream();
        }
        if(name.equals("createMap")) {
            List<Pair<HashCode,HashCode>> values=new ArrayList<>();
            if(args[0] instanceof Map<?,?> input)for(var entry:input.entrySet())values.add(Pair.of((HashCode)entry.getKey(),(HashCode)entry.getValue()));
            else {@SuppressWarnings("unchecked") Stream<Pair<HashCode,HashCode>> input=(Stream<Pair<HashCode,HashCode>>)args[0];values.addAll(input.toList());args[0]=values.stream();}
            List<int[]> entries=new ArrayList<>();for(var pair:values)entries.add(new int[]{node(pair.getFirst()),node(pair.getSecond())});description=map(entries);
        }
        if(name.equals("mergeToMap")) {
            List<int[]> entries=new ArrayList<>();
            if(args.length==3)entries.add(new int[]{child(args[1]),child(args[2])});
            else if(args[1] instanceof Map<?,?> input)for(var entry:input.entrySet())entries.add(new int[]{child(entry.getKey()),child(entry.getValue())});
            else {
                @SuppressWarnings("unchecked") MapLike<HashCode> input=(MapLike<HashCode>)args[1];
                List<Pair<HashCode,HashCode>> values=input.entries().toList();
                for(var pair:values)entries.add(new int[]{child(pair.getFirst()),child(pair.getSecond())});
                args[1]=new MapLike<HashCode>() {
                    public HashCode get(HashCode key) {return input.get(key);}
                    public HashCode get(String key) {return input.get(key);}
                    public Stream<Pair<HashCode,HashCode>> entries() {return values.stream();}
                };
            }
            description=map(entries);
        }
        if(name.equals("mergeToList")) {
            List<Integer> children=new ArrayList<>();
            if(args[1] instanceof List<?> input)for(Object entry:input)children.add(child(entry));
            else children.add(child(args[1]));
            description=list(children);
        }
        if(name.equals("createByteList")) {
            ByteBuffer bytes=((ByteBuffer)args[0]).duplicate();JsonArray values=new JsonArray();while(bytes.hasRemaining())values.add(Byte.toUnsignedInt(bytes.get()));
            description=kind("byte_array");description.add("values",values);
        }
        if(name.equals("createIntList")) {
            int[] values=((IntStream)args[0]).toArray();JsonArray data=new JsonArray();for(int v:values)data.add(v);args[0]=Arrays.stream(values);description=kind("int_array");description.add("values",data);
        }
        if(name.equals("createLongList")) {
            long[] values=((LongStream)args[0]).toArray();JsonArray data=new JsonArray();for(long v:values)data.add(v);args[0]=Arrays.stream(values);description=kind("long_array");description.add("values",data);
        }
        Object result=invokeOriginal(original,method,args);
        if(description!=null) {
            Object produced=value(result);
            if(produced instanceof HashCode code)record(code,description);
            else if(!(result instanceof DataResult<?>))throw new IllegalStateException("native hash factory returned non-hash");
        }
        if(name.equals("mapBuilder"))return builder(result,RecordBuilder.class);
        if(name.equals("listBuilder"))return builder(result,ListBuilder.class);
        Object produced=value(result);
        if(produced instanceof HashCode code && !ledger.containsKey(code))
            throw new IllegalStateException("own recorder missing producing operation "+name);
        return result;
    }
}
