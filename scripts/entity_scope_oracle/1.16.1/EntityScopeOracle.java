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
public final class EntityScopeOracle {
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
    static String[] hooks;
    static double[] shape(Entity entity) {
        double w=0,h=0;
        for(Pose pose:Pose.values()) {
            EntityDimensions d=entity.getDimensions(pose);
            w=Math.max(w,(double)d.width);h=Math.max(h,(double)d.height);
        }
        return new double[]{w,h};
    }
    static JsonArray pair(double[] values) {
        JsonArray a=new JsonArray();for(double x:values)a.add(x);return a;
    }
    static void grow(double[] bound,double[] value) {
        for(int i=0;i<2;i++)bound[i]=Math.max(bound[i],value[i]);
    }
    static JsonObject row(String name,Entity entity,EntityType<?> type,ServerLevel level) {
        JsonObject row=new JsonObject();row.addProperty("name",name);
        EntityDimensions d=type.getDimensions();
        row.add("registry_dimensions",pair(new double[]{(double)d.width,(double)d.height}));
        row.addProperty("class_name",entity.getClass().getName());
        row.addProperty("pushable",entity.isPushable());
        row.addProperty("solid",entity.getCollideBox()!=null);
        row.addProperty("living",entity instanceof LivingEntity);
        double[] base=shape(entity),maximum=base.clone();
        row.add("default_dimensions",pair(base));
        JsonArray cases=new JsonArray();
        try {
            // Probe the native metadata size domains, not a registry-name list.
            Method setter=null;String field=null;int max=0,first=0;
            if(entity instanceof net.minecraft.world.entity.monster.Slime) {
                setter=net.minecraft.world.entity.monster.Slime.class.getDeclaredMethod(hooks[1],int.class,boolean.class);
                setter.setAccessible(true);setter.invoke(entity,Integer.MAX_VALUE,false);
                field="ID_SIZE";max=((net.minecraft.world.entity.monster.Slime)entity).getSize();first=1;
            } else if(entity instanceof net.minecraft.world.entity.monster.Phantom) {
                setter=entity.getClass().getMethod(hooks[2],int.class);setter.invoke(entity,Integer.MAX_VALUE);
                field="ID_SIZE";max=((net.minecraft.world.entity.monster.Phantom)entity).getPhantomSize();
            }
            if(setter!=null) {
                setter.setAccessible(true);
                for(int size=first;size<=Math.min(max,127);size++) {
                    if(setter.getParameterCount()==2)setter.invoke(entity,size,false);else setter.invoke(entity,size);
                    double[] value=shape(entity);grow(maximum,value);
                    JsonObject c=new JsonObject();c.addProperty("value",size);c.add("dimensions",pair(value));cases.add(c);
                }
                if(max>127) {
                    if(setter.getParameterCount()==2)setter.invoke(entity,max,false);else setter.invoke(entity,max);
                    grow(maximum,shape(entity));
                }
                row.addProperty("size_field",field);row.addProperty("size_max",max);
                if(setter.getParameterCount()==2)setter.invoke(entity,1,false);else setter.invoke(entity,0);
            }
            if(entity instanceof Mob) {
                ((Mob)entity).setBaby(true);grow(maximum,shape(entity));((Mob)entity).setBaby(false);
            }
            if(entity instanceof net.minecraft.world.entity.animal.Pufferfish) {
                Method m=net.minecraft.world.entity.animal.Pufferfish.class.getMethod(hooks[3],int.class);
                for(int i=0;i<3;i++){m.invoke(entity,i);grow(maximum,shape(entity));}m.invoke(entity,0);
            }
        } catch(ReflectiveOperationException failure) {throw new RuntimeException(failure);}
        if(entity instanceof net.minecraft.world.entity.animal.PolarBear bear) {
            bear.setNoAi(true);bear.setNoGravity(true);bear.setPos(.5,65.,.5);bear.setStanding(true);
            try {
                Field side=net.minecraft.world.level.Level.class.getDeclaredField(hooks[5]);side.setAccessible(true);
                boolean old=side.getBoolean(level);side.setBoolean(level,true);
                try {for(int i=0;i<7;i++)bear.tick();grow(maximum,shape(bear));}finally {side.setBoolean(level,old);}
            }catch(ReflectiveOperationException failure){throw new RuntimeException(failure);}
        }
        if(entity instanceof net.minecraft.world.entity.decoration.Painting) {
            for(net.minecraft.world.entity.decoration.Motive variant:net.minecraft.core.Registry.MOTIVE)
                grow(maximum,new double[]{variant.getWidth()/16.,variant.getHeight()/16.});
        }
        // The native radius metadata is a float; an isolated cloud has no
        // contact, but a relevant rider must never assume its default radius.
        if(entity instanceof net.minecraft.world.entity.AreaEffectCloud)
            grow(maximum,new double[]{2.*Float.MAX_VALUE,.5});
        row.add("size_cases",cases);row.add("unit_max_dimensions",pair(maximum));
        if(entity instanceof LivingEntity)row.add("scale_limits",pair(new double[]{1.,1.}));
        row.add("max_dimensions",pair(maximum));
        return row;
    }
    public static void main(String[] args) {
        try {
            hooks=args;
            net.minecraft.server.Main.main(new String[]{"--nogui"});
            MinecraftServer server=null;
            for(int i=0;i<120&&(server==null||server.getTickCount()<20);i++){Thread.sleep(500);if(server==null)server=findServer();}
            if(server==null||server.getTickCount()<20)throw new IllegalStateException("server not ready");
            MinecraftServer s=server;JsonArray rows=new JsonArray();
            s.executeBlocking(()->{
                ServerLevel level=s.overworld();
                for(EntityType<?> type:net.minecraft.core.Registry.ENTITY_TYPE) {
                    String name=net.minecraft.core.Registry.ENTITY_TYPE.getKey(type).toString();
                    Entity entity=type.create(level);
                    if(type==EntityType.PLAYER)entity=new ServerPlayer(s,level,new GameProfile(new UUID(0,1),"ScopeOracle"),new net.minecraft.server.level.ServerPlayerGameMode(level));
                    if(type==EntityType.FISHING_BOBBER)entity=new net.minecraft.world.entity.projectile.FishingHook(new ServerPlayer(s,level,new GameProfile(new UUID(0,1),"ScopeOracle"),new net.minecraft.server.level.ServerPlayerGameMode(level)),level,0,0);
                    if(entity==null)throw new IllegalStateException("no constructor: "+name);
                    rows.add(row(name,entity,type,level));
                }
            });
            Files.writeString(Path.of(args[0]),new Gson().toJson(rows)+"\n");s.halt(false);System.exit(0);
        }catch(Throwable failure){failure.printStackTrace();System.exit(1);}
    }
}
