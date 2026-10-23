// Original driver: unchanged native placement context, replacement and cube state methods.
// The generated Level supplies only cell lookups; unrelated world calls fail closed.
import com.google.gson.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import javax.tools.ToolProvider;
public final class ExportReplaceablePlacement {
    static Method replaceMethod(Class<?> c, Class<?> state, Class<?> context) throws Exception {
        for(Class<?> owner=c;owner!=null;owner=owner.getSuperclass())try {
            return owner.getDeclaredMethod("a",state,context);
        } catch(NoSuchMethodException ignored) { }
        throw new NoSuchMethodException("canBeReplaced");
    }
    static String type(Class<?> c) { return c.getCanonicalName(); }
    static Object level(Path work) throws Exception {
        Class<?> level = Class.forName("dwo");
        Constructor<?> ctor = level.getDeclaredConstructors()[0];
        StringBuilder src = new StringBuilder("public class PlacementCells extends dwo { public static java.util.Map<is,eoh> cells; public static eoh air; public PlacementCells() {super(");
        StringJoiner params = new StringJoiner(",");
        for(Class<?> p:ctor.getParameterTypes()) params.add(p==boolean.class?"false":p==long.class?"0L":p==int.class?"0":"("+type(p)+")null");
        src.append(params).append(");}\n public eoh a_(is p){return cells.getOrDefault(p,air);}\n public cee ad(){throw new UnsupportedOperationException();}\n");
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
        src.append("}\nclass SecondaryContext extends dpu {static boolean secondary; public SecondaryContext(dwo w, ddm p, cdb h, dlt s, fti r){super(w,p,h,s,r);} public boolean h(){return secondary;} }\n");
        Path source=work.resolve("PlacementCells.java");Files.writeString(source,src);
        int result=ToolProvider.getSystemJavaCompiler().run(null,null,null,"-proc:none","-cp",System.getProperty("java.class.path"),"-d",work.toString(),source.toString());
        if(result!=0)throw new IllegalStateException("cell provider compilation failed");
        return ExportStorageOutlines.unsafe(new java.net.URLClassLoader(new java.net.URL[]{work.toUri().toURL()}, ExportReplaceablePlacement.class.getClassLoader()).loadClass("PlacementCells"));
    }
    public static void main(String[] args) throws Exception {
        Class.forName("w").getMethod("a").invoke(null);Class.forName("amv").getMethod("a").invoke(null);
        Object world=level(Path.of(args[1]));
        Class<?> carrier=world.getClass().getClassLoader().loadClass("SecondaryContext");
        Class<?> block=Class.forName("dzq"), state=Class.forName("eoh"), base=Class.forName("eog$a"), pos=Class.forName("is"), direction=Class.forName("iz"), vec=Class.forName("ftm"), hit=Class.forName("fti"), context=Class.forName("dpu"), use=Class.forName("dpw"), stack=Class.forName("dlt");
        Object registry=Class.forName("mi").getField("e").get(null);Map<String,Object> blocks=new TreeMap<>();
        for(Object b:(Iterable<?>)registry)blocks.put(Class.forName("jq").getMethod("b",Object.class).invoke(registry,b).toString(),b);
        Object air=block.getMethod("m").invoke(blocks.get("minecraft:air"));world.getClass().getField("air").set(null,air);
        Object clicked=pos.getConstructor(int.class,int.class,int.class).newInstance(2,2,0);
        JsonArray rows=new JsonArray();
        for(String name:List.of("short_grass","fern","dead_bush","stone","water","snow","vine")) {
            Object b=blocks.get("minecraft:"+name), bs=block.getMethod("m").invoke(b);
            for(String material:List.of("stone","dirt","cobblestone","oak_planks","spruce_planks","quartz_block","smooth_quartz","white_concrete","glass","andesite","granite",name))
            for(Object face:(Object[])direction.getMethod("values").invoke(null))
            for(boolean secondary:new boolean[]{false,true}) for(boolean adjacent:new boolean[]{false,true}) for(double hitY:new double[]{2.01,2.4,2.99}) {
                Map<Object,Object> cells=new HashMap<>();
                Object neighbor=pos.getMethod("a",direction).invoke(clicked,face);
                cells.put(clicked,adjacent?block.getMethod("m").invoke(blocks.get("minecraft:stone")):bs);
                if(adjacent)cells.put(neighbor,bs);world.getClass().getField("cells").set(null,cells);
                Object heldItem=block.getMethod("h").invoke(blocks.get("minecraft:"+material));
                Object held=stack.getConstructor(Class.forName("dwn"),int.class).newInstance(heldItem,1);
                Object point=vec.getConstructor(double.class,double.class,double.class).newInstance(2.25,hitY,.5);
                Object result=hit.getConstructor(vec,direction,pos,boolean.class).newInstance(point,face,clicked,false);
                Field secondaryField=carrier.getDeclaredField("secondary");secondaryField.setAccessible(true);secondaryField.setBoolean(null,secondary);
                Constructor<?> c=carrier.getDeclaredConstructor(Class.forName("dwo"),Class.forName("ddm"),Class.forName("cdb"),stack,hit);c.setAccessible(true);
                Object ctx=c.newInstance(world,null,Class.forName("cdb").getField("a").get(null),held,result);
                boolean canReplace=(boolean)base.getMethod("a",context).invoke(bs,ctx);
                Object destination=context.getMethod("a").invoke(ctx);
                Object placed=block.getMethod("a",context).invoke(blocks.get("minecraft:"+(material.equals(name)?"dirt":material)),ctx);
                Object[] pairs={"before",name,"material",material,"face",((Enum<?>)face).ordinal(),"secondary",secondary,"adjacent",adjacent,"hit_y",hitY,
                    "replace_clicked",context.getMethod("c").invoke(ctx),"can_replace",canReplace,"can_place",context.getMethod("b").invoke(ctx),
                    "destination",new int[]{(int)pos.getMethod("u").invoke(destination),(int)pos.getMethod("v").invoke(destination),(int)pos.getMethod("w").invoke(destination)},
                    "placed_state",block.getMethod("j",state).invoke(null,placed),
                    "outline",ExportStorageOutlines.boxes(base.getMethod("f",Class.forName("dvt"),pos).invoke(bs,world,clicked)),
                    "auxiliary",ExportStorageOutlines.boxes(base.getMethod("i",Class.forName("dvt"),pos).invoke(bs,world,clicked)),
                    "collision_empty",ExportStorageOutlines.boxes(base.getMethod("g",Class.forName("dvt"),pos).invoke(bs,world,clicked)).isEmpty(),
                    "liquid",base.getMethod("n").invoke(bs),
                    "replace_owner",replaceMethod(b.getClass(),state,context).getDeclaringClass().getName()};
                Map<String,Object> row=new LinkedHashMap<>();for(int k=0;k<pairs.length;k+=2)row.put((String)pairs[k],pairs[k+1]);
                rows.add(new Gson().toJsonTree(row));
            }
        }
        JsonArray geometry=new JsonArray();
        for(String name:List.of("short_grass","fern","dead_bush")) {
            Object bs=block.getMethod("m").invoke(blocks.get("minecraft:"+name));
            int id=(int)block.getMethod("j",state).invoke(null,bs);
            for(int shift:new int[]{0,2,-8,29999980}) {
                for(int axis=0;axis<3;axis++)for(double edge:new double[]{0,.125,.5,.875,1}) {
                    double[] from={edge,edge,edge},to={edge,edge,edge};from[axis]=-2;to[axis]=2;
                    geometry.add(ExportStorageOutlines.sample(bs,air,id,shift,from,to));
                    geometry.add(ExportStorageOutlines.sample(bs,air,id,shift,to,from));
                }
                geometry.add(ExportStorageOutlines.sample(bs,air,id,shift,new double[]{.5,.5,.5},new double[]{2,2,2}));
                geometry.add(ExportStorageOutlines.sample(bs,air,id,shift,new double[]{-2,-2,-2},new double[]{2,2,2}));
            }
        }
        Files.writeString(Path.of(args[2]),new GsonBuilder().setPrettyPrinting().create().toJson(geometry)+"\n");
        Files.writeString(Path.of(args[0]),new GsonBuilder().setPrettyPrinting().create().toJson(rows)+"\n");
    }
}
