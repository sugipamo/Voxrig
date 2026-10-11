// Own headless harness: original interpolation executes unchanged, with AI/travel isolated.
package voxrig.oracle;
import com.google.gson.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import java.util.concurrent.atomic.AtomicReference;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.entity.*;
import net.minecraft.world.entity.animal.cow.Cow;
import net.minecraft.world.phys.Vec3;
public final class EntityInterpolationOracle {
    static class RemoteCow extends Cow {
        RemoteCow(ServerLevel level){super(EntityType.COW,level);}
        net.minecraft.world.phys.AABB box(Vec3 position){return makeBoundingBox(position);}
    }
    static Field data,steps,previous;
    static int remaining(InterpolationHandler h)throws Exception{return steps.getInt(data.get(h));}
    static MinecraftServer findServer() throws Exception {
        for(Thread thread:Thread.getAllStackTraces().keySet()) {
            if(!thread.getName().equals("Server thread"))continue;
            Field holder=Thread.class.getDeclaredField("holder");holder.setAccessible(true);
            Object h=holder.get(thread);Field task=h.getClass().getDeclaredField("task");task.setAccessible(true);
            Object runnable=task.get(h);
            for(Field f:runnable.getClass().getDeclaredFields()) {
                f.setAccessible(true);
                if(f.get(runnable) instanceof AtomicReference<?> r && r.get() instanceof MinecraftServer s)return s;
            }
        }
        return null;
    }


    static JsonObject pose(Entity e) {
        return pose(e.getX(),e.getY(),e.getZ(),e.getYRot(),e.getXRot());
    }
    static JsonObject pose(double x,double y,double z,float yaw,float pitch) {
        JsonObject o=new JsonObject();JsonArray p=new JsonArray(),r=new JsonArray(),pb=new JsonArray(),rb=new JsonArray();
        for(double n:new double[]{x,y,z}){p.add(n);pb.add(Long.toUnsignedString(Double.doubleToRawLongBits(n),16));}
        for(float n:new float[]{yaw,pitch}){r.add((double)n);rb.add(Integer.toUnsignedLong(Float.floatToRawIntBits(n)));}
        o.add("position",p);o.add("rotation",r);o.add("position_bits",pb);o.add("rotation_bits",rb);return o;
    }
    static double n(JsonObject p,int i){return p.getAsJsonArray("position").get(i).getAsDouble();}
    static float r(JsonObject p,int i){return p.getAsJsonArray("rotation").get(i).getAsFloat();}
    static JsonObject target(int scenario) {
        double[][] positions={{3.1,67.3,-2.7},{0,65,0},{1e6,65.1,-1e6},{-0.,65,-0.},{.0000001,65.0000001,-.0000001}};
        float[][] rotations={{179,-179},{-179,179},{180,0},{-180,-0.f},{720.125f,-721.75f},{Float.intBitsToFloat(0x3f800001),.1f},{-0.f,-0.f}};
        double[] p=positions[scenario%positions.length];float[] r=rotations[scenario%rotations.length];
        return pose(p[0],p[1],p[2],r[0],r[1]);
    }
    static JsonObject evaluate(int scenario,int length,MinecraftServer s,ServerLevel level) throws Exception {
        RemoteCow e=new RemoteCow(level);InterpolationHandler h=e.getInterpolation();h.setInterpolationLength(length);
        JsonObject initial=pose(.1,65.2,-.3,scenario%2==0?170:-170,scenario%3==0?45:-45);
        e.setPos(n(initial,0),n(initial,1),n(initial,2));e.setYRot(r(initial,0));e.setXRot(r(initial,1));
        JsonArray events=new JsonArray();JsonObject a=target(scenario);
        for(int index=0;index<9;index++) {
            String op=index==0||index==2||index==4?"target":index==7?"cancel":"step";
            JsonObject event=new JsonObject();event.addProperty("operation",op);
            if(op.equals("target")) {
                if(index==2&&scenario%2==1)e.setPos(e.getX()+.375,e.getY(),e.getZ());
                event.add("before",pose(e));
                JsonObject t=index==4?target(scenario+1):a;event.add("input",t);h.interpolateTo(new Vec3(n(t,0),n(t,1),n(t,2)),r(t,0),r(t,1));
            } else if(op.equals("cancel")) { h.cancel(); }
            else {
                if(index==3) {
                    JsonObject before=pose(e);e.setPos(e.getX()+.25,e.getY()+.125,e.getZ()-.5);e.setYRot(e.getYRot()+10);e.setXRot(e.getXRot()-5);
                }
                JsonObject before=pose(e);event.add("input",before);
                if(h.hasActiveInterpolation()) {
                    // Read the actual original baseline, including identical-target admission.
                    Vec3 delta=e.position().subtract((Vec3)previous.get(h));
                    Vec3 shifted=h.position().add(delta);
                    var block=net.minecraft.core.BlockPos.containing(shifted);
                    boolean blocked=scenario%2==1;
                    level.setBlock(block,blocked?net.minecraft.world.level.block.Blocks.STONE.defaultBlockState():net.minecraft.world.level.block.Blocks.AIR.defaultBlockState(),0);
                    boolean free=level.noCollision(e,e.box(shifted));
                    event.addProperty("collision_free",free);
                    h.interpolate();
                    level.setBlock(block,net.minecraft.world.level.block.Blocks.AIR.defaultBlockState(),0);
                }else h.interpolate();
                
            }
            event.add("pose",pose(e));event.addProperty("remaining_steps",remaining(h));
            events.add(event);
        }
        JsonObject row=new JsonObject();row.addProperty("scenario",scenario);row.addProperty("length",length);
        row.add("initial",initial);row.add("events",events);return row;
    }
    public static void main(String[] args) {
        try {
            data=InterpolationHandler.class.getDeclaredField(args[1]);data.setAccessible(true);steps=Class.forName(InterpolationHandler.class.getName()+"$"+args[3]).getDeclaredField(args[2]);steps.setAccessible(true);previous=InterpolationHandler.class.getDeclaredField(args[4]);previous.setAccessible(true);
            net.minecraft.server.Main.main(new String[]{"--nogui"});MinecraftServer server=null;
            for(int i=0;i<120&&(server==null||server.getTickCount()<20);i++){Thread.sleep(500);if(server==null)server=findServer();}
            if(server==null||server.getTickCount()<20)throw new IllegalStateException("original server not ready");
            MinecraftServer s=server;JsonArray rows=new JsonArray();
            s.executeBlocking(()->{try{for(int length:new int[]{0,1,2,3,7,120})for(int scenario=0;scenario<35;scenario++)rows.add(evaluate(scenario,length,s,s.overworld()));}catch(Exception e){throw new RuntimeException(e);}});
            Files.writeString(Path.of(args[0]),new Gson().toJson(rows)+"\n");s.halt(false);System.exit(0);
        }catch(Throwable e){e.printStackTrace();System.exit(1);}
    }
}
