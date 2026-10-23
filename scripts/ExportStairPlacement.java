// Calls original stair placement, shape and outline methods; only cell/input carriers are supplied.
import com.google.gson.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import javax.tools.ToolProvider;
public final class ExportStairPlacement {
    static String type(Class<?> c) { return c.getCanonicalName(); }
    static Object level(Path work) throws Exception {
        Class<?> level = Class.forName("dwo");
        Constructor<?> ctor = level.getDeclaredConstructors()[0];
        StringBuilder src = new StringBuilder("public class StairCells extends dwo { public static java.util.Map<is,eoh> cells; public static eoh air; public StairCells() {super(");
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
        src.append("}\nclass StairContext extends dpu {static boolean secondary; static float yaw; public StairContext(dwo w, ddm p, cdb h, dlt s, fti r){super(w,p,h,s,r);} public boolean h(){return secondary;} public iz g(){return iz.a((double)yaw);} }\n");
        Path source=work.resolve("StairCells.java");Files.writeString(source,src);
        int result=ToolProvider.getSystemJavaCompiler().run(null,null,null,"-proc:none","-cp",System.getProperty("java.class.path"),"-d",work.toString(),source.toString());
        if(result!=0)throw new IllegalStateException("cell provider compilation failed");
        return ExportStorageOutlines.unsafe(new java.net.URLClassLoader(new java.net.URL[]{work.toUri().toURL()}, ExportStairPlacement.class.getClassLoader()).loadClass("StairCells"));
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
    static Object at(int x,int y,int z) throws Exception {return Class.forName("is").getConstructor(int.class,int.class,int.class).newInstance(x,y,z);}
    public static void main(String[] args) throws Exception {
        Class.forName("w").getMethod("a").invoke(null);Class.forName("amv").getMethod("a").invoke(null);
        Object world=level(Path.of(args[1]));Class<?> carrier=world.getClass().getClassLoader().loadClass("StairContext");
        Class<?> block=Class.forName("dzq"), state=Class.forName("eoh"), pos=Class.forName("is"), direction=Class.forName("iz"), context=Class.forName("dpu");
        Object registry=Class.forName("mi").getField("e").get(null);Map<String,Object> blocks=new TreeMap<>();
        for(Object b:(Iterable<?>)registry)blocks.put(Class.forName("jq").getMethod("b",Object.class).invoke(registry,b).toString(),b);
        Object air=block.getMethod("m").invoke(blocks.get("minecraft:air")), stone=block.getMethod("m").invoke(blocks.get("minecraft:stone"));
        world.getClass().getField("air").set(null,air);
        Object oak=blocks.get("minecraft:oak_stairs"), origin=at(0,2,0);List<Object> dry=new ArrayList<>(), straight=new ArrayList<>();
        for(Object bs:(List<?>)Class.forName("eoi").getMethod("a").invoke(block.getMethod("l").invoke(oak))) {
            JsonObject json=stateJson(bs,"minecraft:oak_stairs");
            if(json.getAsJsonObject("properties").get("waterlogged").getAsString().equals("false")) {
                dry.add(bs);if(json.getAsJsonObject("properties").get("shape").getAsString().equals("straight"))straight.add(bs);
            }
        }
        Method shape=Class.forName("ehz").getDeclaredMethod("e",state,Class.forName("dvt"),pos);shape.setAccessible(true);
        Object property=Class.forName("ehz").getField("d").get(null);
        Method set=Class.forName("eoj").getMethod("b",Class.forName("epk"),Comparable.class);
        JsonArray shapes=new JsonArray(),placements=new JsonArray(),geometry=new JsonArray();
        int[][] offsets={{0,0,1},{-1,0,0},{0,0,-1},{1,0,0}};
        List<Object> choices=new ArrayList<>();choices.add(air);choices.addAll(straight);
        for(Object center:straight)for(int a=0;a<9;a++)for(int b=0;b<9;b++)for(int c=0;c<9;c++)for(int d=0;d<9;d++) {
            Map<Object,Object> cells=new HashMap<>();JsonArray neighbors=new JsonArray();int[] indices={a,b,c,d};
            for(int i=0;i<4;i++){Object value=choices.get(indices[i]);cells.put(at(offsets[i][0],2,offsets[i][2]),value);neighbors.add(stateJson(value,indices[i]==0?"minecraft:air":"minecraft:oak_stairs"));}
            world.getClass().getField("cells").set(null,cells);
            Object expected=set.invoke(center,property,shape.invoke(null,center,world,origin));
            JsonObject row=new JsonObject();row.add("state",stateJson(center,"minecraft:oak_stairs"));row.add("neighbors",neighbors);row.add("expected",stateJson(expected,"minecraft:oak_stairs"));shapes.add(row);
        }
        Object held=Class.forName("dlt").getConstructor(Class.forName("dwn"),int.class).newInstance(block.getMethod("h").invoke(oak),1);
        Field yawField=carrier.getDeclaredField("yaw");yawField.setAccessible(true);
        Constructor<?> ctor=carrier.getDeclaredConstructor(Class.forName("dwo"),Class.forName("ddm"),Class.forName("cdb"),Class.forName("dlt"),Class.forName("fti"));ctor.setAccessible(true);
        List<Float> yaws=new ArrayList<>();for(float yaw:new float[]{-315,-225,-135,-45,45,135,225,315})for(float v:new float[]{Math.nextDown(yaw),yaw,Math.nextUp(yaw)})yaws.add(v);
        for(float yaw:new float[]{-180,-90,0,90,180,270,360,-Float.MAX_VALUE,Float.MAX_VALUE})yaws.add(yaw);
        for(float yaw:yaws)for(Object face:(Object[])direction.getMethod("values").invoke(null))for(float y:new float[]{0,.25f,Math.nextDown(.5f),.5f,Math.nextUp(.5f),.75f,1}) {
            Object clicked=pos.getMethod("a",direction).invoke(origin,direction.getMethod("g").invoke(face));Map<Object,Object> cells=new HashMap<>();cells.put(clicked,stone);world.getClass().getField("cells").set(null,cells);
            Object point=Class.forName("ftm").getConstructor(double.class,double.class,double.class).newInstance(.25,2.0+(double)y,.5);
            Object hit=Class.forName("fti").getConstructor(Class.forName("ftm"),direction,pos,boolean.class).newInstance(point,face,clicked,false);
            yawField.setFloat(null,yaw);Object ctx=ctor.newInstance(world,null,Class.forName("cdb").getField("a").get(null),held,hit);
            Object placed=block.getMethod("a",context).invoke(oak,ctx);
            JsonObject row=new JsonObject();row.addProperty("yaw",yaw);row.addProperty("face",((Enum<?>)face).ordinal());row.addProperty("hit_y",y);row.add("expected",stateJson(placed,"minecraft:oak_stairs"));placements.add(row);
        }
        for(Object bs:dry) {
            int id=(int)block.getMethod("j",state).invoke(null,bs);
            for(int shift:new int[]{0,29999980})for(double edge:new double[]{0,.25,.5,.75,1}) {
                for(int axis=0;axis<3;axis++){double[] from={edge,edge,edge},to=from.clone();from[axis]=-2;to[axis]=2;
                    geometry.add(ExportStorageOutlines.sample(bs,air,id,shift,from,to));geometry.add(ExportStorageOutlines.sample(bs,air,id,shift,to,from));}
                geometry.add(ExportStorageOutlines.sample(bs,air,id,shift,new double[]{-2,edge,-2},new double[]{2,edge,2}));
                geometry.add(ExportStorageOutlines.sample(bs,air,id,shift,new double[]{2,edge,-2},new double[]{-2,edge,2}));
            }
        }
        Gson gson=new Gson();Files.writeString(Path.of(args[0]),gson.toJson(shapes)+"\n");Files.writeString(Path.of(args[2]),gson.toJson(placements)+"\n");Files.writeString(Path.of(args[3]),gson.toJson(geometry)+"\n");
    }
}
