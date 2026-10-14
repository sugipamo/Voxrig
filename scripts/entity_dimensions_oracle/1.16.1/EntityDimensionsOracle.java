// Own harness: calls unchanged original entity dimension methods in an original ServerLevel.
package voxrig.oracle;
import com.google.gson.*;
import com.mojang.authlib.GameProfile;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import java.util.concurrent.atomic.AtomicReference;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.entity.*;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.entity.animal.Cow;
import net.minecraft.world.entity.animal.Pig;
import net.minecraft.world.entity.monster.Zombie;
public final class EntityDimensionsOracle {
    static Method bounds;
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
    static JsonObject row(String type,LivingEntity entity,Pose pose,boolean baby,double requested) {
        EntityDimensions d=entity.getDimensions(pose);
        AABB b; try {entity.setPos(.5,65.,2.5);b=(AABB)bounds.invoke(entity,pose);} catch(Exception e) {throw new RuntimeException(e);}
        JsonObject row=new JsonObject();row.addProperty("type",type);row.addProperty("pose",pose.ordinal());
        row.addProperty("default_pose",entity.getPose().ordinal());row.addProperty("baby",baby);row.addProperty("requested_scale",requested);
        row.addProperty("effective_scale",1.);
        row.addProperty("width",(double)d.width);row.addProperty("height",(double)d.height);
        JsonArray bounds=new JsonArray();for(double n:new double[]{b.minX,b.minY,b.minZ,b.maxX,b.maxY,b.maxZ})bounds.add(n);
        row.add("bounds",bounds);
        row.addProperty("width_bits",Integer.toUnsignedLong(Float.floatToRawIntBits(d.width)));
        row.addProperty("height_bits",Integer.toUnsignedLong(Float.floatToRawIntBits(d.height)));
        row.addProperty("effective_scale_bits",Integer.toUnsignedLong(Float.floatToRawIntBits(1f)));
        JsonArray bits=new JsonArray();for(double n:new double[]{b.minX,b.minY,b.minZ,b.maxX,b.maxY,b.maxZ})bits.add(Long.toUnsignedString(Double.doubleToRawLongBits(n),16));
        row.add("bounds_bits",bits);return row;
    }
    public static void main(String[] args) {
        try {
            bounds=Entity.class.getDeclaredMethod(args[1],Pose.class);bounds.setAccessible(true);
            net.minecraft.server.Main.main(new String[]{"--nogui"});
            MinecraftServer server=null;
            for(int i=0;i<120&&(server==null||server.getTickCount()<20);i++) {Thread.sleep(500);if(server==null)server=findServer();}
            if(server==null||server.getTickCount()<20)throw new IllegalStateException("original server not ready");
            MinecraftServer s=server;JsonArray rows=new JsonArray();
            s.executeBlocking(()-> {
                ServerLevel level=s.overworld();
                ServerPlayer player=new ServerPlayer(s,level,new GameProfile(new UUID(0,1),"DimensionsOracle"),new net.minecraft.server.level.ServerPlayerGameMode(level));
                LivingEntity[] entities={player,new Cow(EntityType.COW,level),new Pig(EntityType.PIG,level),new Zombie(EntityType.ZOMBIE,level)};
                String[] names={"minecraft:player","minecraft:cow","minecraft:pig","minecraft:zombie"};
                for(int i=0;i<entities.length;i++) {
                    LivingEntity entity=entities[i];
                    for(boolean baby:new boolean[]{false,true}) {
                        if(i==0&&baby)continue;
                        if(i==1||i==2)((AgableMob)entity).setBaby(baby);
                        if(i==3)((Zombie)entity).setBaby(baby);
                        for(double scale:new double[]{1.}) {
                            
                            for(Pose pose:Pose.values())rows.add(row(names[i],entity,pose,baby,scale));
                        }
                    }
                }
            });
            Files.writeString(Path.of(args[0]),new GsonBuilder().create().toJson(rows)+"\n");
            s.halt(false);System.exit(0);
        } catch(Throwable failure) {failure.printStackTrace();System.exit(1);}
    }
}
