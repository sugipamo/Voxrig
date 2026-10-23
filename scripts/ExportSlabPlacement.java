// Original driver: unchanged native placement context, slab replacement and state methods.
// The generated Level supplies only cell/fluid lookups; unrelated world calls fail closed.
import com.google.gson.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import javax.tools.ToolProvider;
public final class ExportSlabPlacement {
    static String type(Class<?> c) { return c.getCanonicalName(); }
    static Object level(Path work) throws Exception {
        Class<?> level = Class.forName("dwo");
        Constructor<?> ctor = level.getDeclaredConstructors()[0];
        StringBuilder src = new StringBuilder("public class SlabCells extends dwo { public static java.util.Map<is,eoh> cells; public static eoh air; public SlabCells() {super(");
        StringJoiner params = new StringJoiner(",");
        for(Class<?> p:ctor.getParameterTypes()) params.add(p==boolean.class?"false":p==long.class?"0L":p==int.class?"0":"("+type(p)+")null");
        src.append(params).append(");}\n public eoh a_(is p){return cells.getOrDefault(p,air); } public flb b_(is p){return a_(p).y();}\n public cee ad(){throw new UnsupportedOperationException();}\n");
        Set<String> emitted = new HashSet<>();
        Map<String,Method> methods=new LinkedHashMap<>();
        for(Class<?> current=level;current!=null;current=current.getSuperclass())for(Method m:current.getDeclaredMethods())
            if(!m.isBridge())methods.putIfAbsent(m.getName()+Arrays.toString(m.getParameterTypes()),m);
        for(Method m:level.getMethods())if(!m.isBridge())methods.putIfAbsent(m.getName()+Arrays.toString(m.getParameterTypes()),m);
        for(Class<?> iface:level.getInterfaces())for(Method m:iface.getMethods())if(!m.isBridge())methods.putIfAbsent(m.getName()+Arrays.toString(m.getParameterTypes()),m);
        for(Method m:methods.values()) if(Modifier.isAbstract(m.getModifiers())) {
            String signature=m.getName()+Arrays.toString(m.getParameterTypes());
            if(!emitted.add(signature))continue;
            src.append("public ").append(type(m.getReturnType())).append(" ").append(m.getName()).append("(");
            StringJoiner args=new StringJoiner(",");int i=0;for(Class<?> p:m.getParameterTypes())args.add(type(p)+" p"+(i++));
            src.append(args).append("){throw new UnsupportedOperationException(\"unexpected world call\");}\n");
        }
        src.append("}\nclass SlabContext extends dpu {static boolean secondary; public SlabContext(dwo w, ddm p, cdb h, dlt s, fti r){super(w,p,h,s,r);} public boolean h(){return secondary;} }\n");
        Path source=work.resolve("SlabCells.java");Files.writeString(source,src);
        int result=ToolProvider.getSystemJavaCompiler().run(null,null,null,"-proc:none","-cp",System.getProperty("java.class.path"),"-d",work.toString(),source.toString());
        if(result!=0)throw new IllegalStateException("cell provider compilation failed");
        return ExportStorageOutlines.unsafe(new java.net.URLClassLoader(new java.net.URL[]{work.toUri().toURL()}, ExportSlabPlacement.class.getClassLoader()).loadClass("SlabCells"));
    }
    static JsonObject stateJson(Object value, String name) throws Exception {
        JsonObject out=new JsonObject(), properties=new JsonObject();out.addProperty("name",name);
        Map<?,?> entries=(Map<?,?>)Class.forName("eoj").getMethod("G").invoke(value);
        Map<String,String> ordered=new TreeMap<>();
        for(Map.Entry<?,?> e:entries.entrySet()) {
            Object property=e.getKey();
            ordered.put((String)Class.forName("epk").getMethod("f").invoke(property),
                (String)Class.forName("epk").getMethod("b",Comparable.class).invoke(property,e.getValue()));
        }
        ordered.forEach(properties::addProperty);out.add("properties",properties);return out;
    }
    public static void main(String[] args) throws Exception {
        Class.forName("w").getMethod("a").invoke(null);Class.forName("amv").getMethod("a").invoke(null);
        Object world=level(Path.of(args[1]));
        Class<?> carrier=world.getClass().getClassLoader().loadClass("SlabContext");
        Class<?> block=Class.forName("dzq"), state=Class.forName("eoh"), base=Class.forName("eog$a"), pos=Class.forName("is"), direction=Class.forName("iz"), vec=Class.forName("ftm"), hit=Class.forName("fti"), context=Class.forName("dpu"), stack=Class.forName("dlt");
        Object registry=Class.forName("mi").getField("e").get(null);Map<String,Object> blocks=new TreeMap<>();
        for(Object b:(Iterable<?>)registry)blocks.put(Class.forName("jq").getMethod("b",Object.class).invoke(registry,b).toString(),b);
        Object air=block.getMethod("m").invoke(blocks.get("minecraft:air"));world.getClass().getField("air").set(null,air);
        Object clicked=pos.getConstructor(int.class,int.class,int.class).newInstance(2,2,0);
        Object oak=blocks.get("minecraft:oak_slab");
        JsonArray rows=new JsonArray(),geometry=new JsonArray(),obstruction=new JsonArray();
        for(String name:List.of("air","stone","oak_slab","spruce_slab","water")) {
            Object b=blocks.get("minecraft:"+name), definition=block.getMethod("l").invoke(b);
            for(Object bs:(List<?>)Class.forName("eoi").getMethod("a").invoke(definition)) {
                JsonObject before=stateJson(bs,"minecraft:"+name);
                // Water's level variants are outside this dry API; retain the source state.
                if(name.equals("water")&&!before.getAsJsonObject("properties").get("level").getAsString().equals("0"))continue;
                for(Object face:(Object[])direction.getMethod("values").invoke(null))
                for(boolean secondary:new boolean[]{false,true}) for(boolean adjacent:new boolean[]{false,true})
                for(float hitY:new float[]{0f,.25f,Math.nextDown(.5f),.5f,Math.nextUp(.5f),.75f,1f}) {
                    Map<Object,Object> cells=new HashMap<>();Object neighbor=pos.getMethod("a",direction).invoke(clicked,face);
                    cells.put(clicked,adjacent?block.getMethod("m").invoke(blocks.get("minecraft:stone")):bs);
                    if(adjacent)cells.put(neighbor,bs);world.getClass().getField("cells").set(null,cells);
                    Object held=stack.getConstructor(Class.forName("dwn"),int.class).newInstance(block.getMethod("h").invoke(oak),1);
                    Object point=vec.getConstructor(double.class,double.class,double.class).newInstance(2.25,2.0+(double)hitY,.5);
                    Object result=hit.getConstructor(vec,direction,pos,boolean.class).newInstance(point,face,clicked,false);
                    Field secondaryField=carrier.getDeclaredField("secondary");secondaryField.setAccessible(true);secondaryField.setBoolean(null,secondary);
                    Constructor<?> c=carrier.getDeclaredConstructor(Class.forName("dwo"),Class.forName("ddm"),Class.forName("cdb"),stack,hit);c.setAccessible(true);
                    Object ctx=c.newInstance(world,null,Class.forName("cdb").getField("a").get(null),held,result);
                    Object destination=context.getMethod("a").invoke(ctx),destinationState=world.getClass().getMethod("a_",pos).invoke(world,destination);
                    Object placed=block.getMethod("a",context).invoke(oak,ctx);
                    Object[] pairs={"before",before,"face",((Enum<?>)face).ordinal(),"secondary",secondary,"adjacent",adjacent,"hit_y",hitY,
                        "replace_clicked",context.getMethod("c").invoke(ctx),"can_place",context.getMethod("b").invoke(ctx),
                        "destination",new int[]{(int)pos.getMethod("u").invoke(destination),(int)pos.getMethod("v").invoke(destination),(int)pos.getMethod("w").invoke(destination)},
                        "destination_state",block.getMethod("j",state).invoke(null,destinationState),
                        "placed_state",stateJson(placed,"minecraft:oak_slab"),
                        "collision",ExportStorageOutlines.boxes(base.getMethod("g",Class.forName("dvt"),pos).invoke(placed,world,destination))};
                    Map<String,Object> row=new LinkedHashMap<>();for(int k=0;k<pairs.length;k+=2)row.put((String)pairs[k],pairs[k+1]);rows.add(new Gson().toJsonTree(row));
                }
                if(name.equals("oak_slab")) {
                    if(before.getAsJsonObject("properties").get("waterlogged").getAsString().equals("false")) {
                        Object collision=base.getMethod("g",Class.forName("dvt"),pos).invoke(bs,world,clicked);
                        for(double low:new double[]{-.1,0.,.25,.5,.5001,.75,1.,1.1})for(double height:new double[]{.1,.25,1.8})
                        for(double x:new double[]{-.5,.25,1.}) {
                            double[] box={x,low,.25,x+.5,low+height,.75};
                            Object body=Class.forName("fud").getMethod("b",double.class,double.class,double.class,double.class,double.class,double.class)
                                .invoke(null,box[0],box[1],box[2],box[3],box[4],box[5]);
                            boolean overlap=(boolean)Class.forName("fud").getMethod("c",Class.forName("fug"),Class.forName("fug"),Class.forName("ftq"))
                                .invoke(null,collision,body,Class.forName("ftq").getField("i").get(null));
                            JsonObject row=new JsonObject();row.add("state",before);row.add("body",ExportStorageOutlines.numbers(box));
                            row.addProperty("intersects",overlap);obstruction.add(row);
                        }
                    }
                    int id=(int)block.getMethod("j",state).invoke(null,bs);
                    for(int shift:new int[]{0,29999980})for(int axis=0;axis<3;axis++)for(double edge:new double[]{0,.25,.5,.75,1}) {
                        double[] from={edge,edge,edge},to=from.clone();from[axis]=-2;to[axis]=2;
                        geometry.add(ExportStorageOutlines.sample(bs,air,id,shift,from,to));
                        geometry.add(ExportStorageOutlines.sample(bs,air,id,shift,to,from));
                    }
                }
            }
        }
        Files.writeString(Path.of(args[3]),new GsonBuilder().setPrettyPrinting().create().toJson(obstruction)+"\n");
        Files.writeString(Path.of(args[0]),new GsonBuilder().setPrettyPrinting().create().toJson(rows)+"\n");
        Files.writeString(Path.of(args[2]),new GsonBuilder().setPrettyPrinting().create().toJson(geometry)+"\n");
    }
}
