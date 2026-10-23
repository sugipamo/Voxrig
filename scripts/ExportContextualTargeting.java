// Original driver for unchanged 1.21.11 Avatar dimensions, Entity view vector,
// and BlockGetter.clip OUTLINE/NONE. No server or world is started.
import com.google.gson.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;

public final class ExportContextualTargeting {
    static final Gson JSON = new GsonBuilder().serializeNulls().create();
    static double[] vector(Object value) throws Exception {
        Class<?> type = Class.forName("ftm");
        return new double[]{type.getField("g").getDouble(value), type.getField("h").getDouble(value), type.getField("i").getDouble(value)};
    }
    static List<Integer> cell(int x, int y, int z) { return List.of(x,y,z); }
    public static void main(String[] args) throws Exception {
        Class.forName("w").getMethod("a").invoke(null);
        Class.forName("amv").getMethod("a").invoke(null);
        ExportStorageOutlines.legacy = false;
        Class<?> avatar = Class.forName("cgc"), poseClass = Class.forName("chx"), dimensions = Class.forName("cgn");
        Field poses = avatar.getDeclaredField("r"); poses.setAccessible(true);
        Map<?,?> table = (Map<?,?>)poses.get(null);
        Method viewVector = Class.forName("cgk").getDeclaredMethod("c", float.class, float.class); viewVector.setAccessible(true);
        Object player = ExportStorageOutlines.unsafe(avatar); // Pure view-vector method reads no entity fields.
        Class<?> block = Class.forName("dzq"), blockState = Class.forName("eoh");
        Class<?> registryClass = Class.forName("jq"), pos = Class.forName("is"), view = Class.forName("dvt"), vec = Class.forName("ftm");
        Object registry = Class.forName("mi").getField("e").get(null);
        Method key = registryClass.getMethod("b",Object.class);
        Map<String,Object> blocks = new HashMap<>();
        for(Object b:(Iterable<?>)registry) blocks.put(key.invoke(registry,b).toString(), b);
        Object air = block.getMethod("m").invoke(blocks.get("minecraft:air"));
        Object stone = block.getMethod("m").invoke(blocks.get("minecraft:stone"));
        Object chest = block.getMethod("m").invoke(blocks.get("minecraft:chest"));
        Object scaleAttribute = Class.forName("jd").getMethod("a").invoke(Class.forName("cis").getField("A").get(null));
        Object reachAttribute = Class.forName("jd").getMethod("a").invoke(Class.forName("cis").getField("h").get(null));
        Class<?> ranged = Class.forName("ciu");
        double[] scaleLimits = {(Double)ranged.getMethod("d").invoke(scaleAttribute),(Double)ranged.getMethod("e").invoke(scaleAttribute)};
        double[] reachLimits = {(Double)ranged.getMethod("d").invoke(reachAttribute),(Double)ranged.getMethod("e").invoke(reachAttribute)};
        JsonArray cases = new JsonArray();
        for(int poseId:new int[]{0,5}) for(double scale:new double[]{.125,.25,.5,1.,1.3,2.,4.})
        for(double reach:new double[]{0.,.5,4.5,8.}) for(float[] rotation:new float[][]{{0,0},{0,90},{-45,0},{45,45},{0,30},{90,0}})
        for(int scene=0;scene<4;scene++) {
            Object nativePose = poseClass.getField(poseId==0?"a":"f").get(null);
            Object d = dimensions.getMethod("a",float.class).invoke(table.get(nativePose),(float)scale);
            double eyeHeight = ((Float)dimensions.getMethod("c").invoke(d)).doubleValue();
            double[] position = {.5,181.,.5}, eye = {.5,181.+eyeHeight,.5};
            Object box = dimensions.getMethod("a",double.class,double.class,double.class).invoke(d,position[0],position[1],position[2]);
            double[] bounds = new double[6];
            for(int i=0;i<6;i++)bounds[i]=box.getClass().getField("abcdef".substring(i,i+1)).getDouble(box);
            double[] direction = vector(viewVector.invoke(player, rotation[1],rotation[0]));
            double[] end = new double[3];for(int i=0;i<3;i++)end[i]=eye[i]+direction[i]*reach;
            Map<List<Integer>,Object> cells = new LinkedHashMap<>();
            for(int x=-2;x<=2;x++)for(int z=-2;z<=2;z++)cells.put(cell(x,180,z),stone);
            if(scene==0)for(int y=181;y<=188;y++)cells.put(cell(0,y,4),stone);
            if(scene==1)cells.put(cell(0,(int)Math.floor(eye[1]),2),chest);
            if(scene==2) {cells.put(cell(1,(int)Math.floor(eye[1]),1),stone);cells.put(cell(2,(int)Math.floor(eye[1]),2),chest);}
            if(scene==3)cells.put(cell(0,(int)Math.floor(eye[1]),0),stone); // Inside clip; geometric fixture only.
            Object world=Proxy.newProxyInstance(view.getClassLoader(),new Class<?>[]{view},(proxy,method,a)->{
                if(method.isDefault())return InvocationHandler.invokeDefault(proxy,method,a);
                Object state=air;
                if(a!=null&&a.length>0&&pos.isInstance(a[0])) {
                    int x=(Integer)pos.getMethod("u").invoke(a[0]),y=(Integer)pos.getMethod("v").invoke(a[0]),z=(Integer)pos.getMethod("w").invoke(a[0]);
                    state=cells.getOrDefault(cell(x,y,z),air);
                }
                return switch(method.getName()) {
                    case "a_" -> state;
                    case "b_" -> Class.forName("eog$a").getMethod("y").invoke(state);
                    case "c_" -> null;
                    case "L_" -> 384;
                    case "K_" -> -64;
                    default -> throw new IllegalStateException(method.toString());
                };
            });
            Object start=vec.getConstructor(double.class,double.class,double.class).newInstance(eye[0],eye[1],eye[2]);
            Object finish=vec.getConstructor(double.class,double.class,double.class).newInstance(end[0],end[1],end[2]);
            Object hit=view.getMethod("a",Class.forName("dvw")).invoke(world,ExportStorageOutlines.context(start,finish));
            Map<String,Object> row=new LinkedHashMap<>();
            row.put("scale_limits",scaleLimits);row.put("reach_limits",reachLimits);
            row.put("pose",poseId);row.put("scale",scale);row.put("reach",reach);row.put("rotation",rotation);
            row.put("position",position);row.put("eye",eye);row.put("bounds",bounds);row.put("scene",scene);
            row.put("eye_bits",Arrays.stream(eye).mapToObj(v->Long.toHexString(Double.doubleToRawLongBits(v))).toList());
            row.put("bounds_bits",Arrays.stream(bounds).mapToObj(v->Long.toHexString(Double.doubleToRawLongBits(v))).toList());
            List<Object> encoded=new ArrayList<>();
            for(var entry:cells.entrySet())encoded.add(Map.of("position",entry.getKey(),"state",block.getMethod("j",blockState).invoke(null,entry.getValue())));
            row.put("cells",encoded);
            if(hit.getClass().getMethod("d").invoke(hit).toString().equals("BLOCK")) {
                Object p=hit.getClass().getMethod("b").invoke(hit);
                row.put("hit",Map.of("position",new int[]{(Integer)pos.getMethod("u").invoke(p),(Integer)pos.getMethod("v").invoke(p),(Integer)pos.getMethod("w").invoke(p)},
                    "point",vector(hit.getClass().getMethod("g").invoke(hit)),
                    "face",hit.getClass().getMethod("c").invoke(hit).toString().toLowerCase(Locale.ROOT)));
            } else row.put("hit",null);
            cases.add(JSON.toJsonTree(row));
        }
        Files.writeString(Path.of(args[0]),JSON.toJson(cases)+"\n");
    }
}
