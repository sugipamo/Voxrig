// Own harness: calls unchanged original entity pair push methods in an original ServerLevel.
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
import net.minecraft.world.entity.animal.cow.Cow;
import net.minecraft.world.entity.animal.pig.Pig;
import net.minecraft.world.entity.monster.zombie.Zombie;
public final class EntityPushOracle {
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

    static Field gameMode,vehicle,passengers;
    static void riding(Entity rider,Entity root) {
        try {
            vehicle.set(rider,root);
            var members=new java.util.ArrayList<Entity>((java.util.List<Entity>)passengers.get(root));members.add(rider);
            passengers.set(root,com.google.common.collect.ImmutableList.copyOf(members));
            if(rider.getVehicle()!=root||!root.hasPassenger(rider))throw new IllegalStateException("native relation setup failed");
        }catch(Exception e){throw new RuntimeException(e);}
    }
    static void spectator(ServerPlayer p) {try {gameMode.set(p.gameMode,net.minecraft.world.level.GameType.SPECTATOR);}catch(Exception e){throw new RuntimeException(e);}}
    static LivingEntity body(int kind,MinecraftServer s,ServerLevel level) {
        return switch(kind) {
            case 0 -> new ServerPlayer(s,level,new GameProfile(UUID.randomUUID(),"PushOracle"),net.minecraft.server.level.ClientInformation.createDefault());
            case 1 -> new Cow(EntityType.COW,level);
            case 2 -> new Pig(EntityType.PIG,level);
            default -> new Zombie(EntityType.ZOMBIE,level);
        };
    }
    static JsonArray vector(double[] v) {JsonArray a=new JsonArray();for(double n:v)a.add(n);return a;}
    static JsonArray bits(double[] v) {JsonArray a=new JsonArray();for(double n:v)a.add(Long.toUnsignedString(Double.doubleToRawLongBits(n),16));return a;}
    static double[] velocity(Entity e) {var v=e.getDeltaMovement();return new double[]{v.x,v.y,v.z};}
    static JsonObject inputBody(LivingEntity e) {
        JsonObject o=new JsonObject();double[] p={e.getX(),e.getY(),e.getZ()};double[] v=velocity(e);
        o.add("position",vector(p));o.add("position_bits",bits(p));o.add("velocity",vector(v));o.add("velocity_bits",bits(v));
        o.addProperty("no_physics",e.noPhysics);o.addProperty("vehicle",e.isVehicle());o.addProperty("pushable",e.isPushable());return o;
    }
    static JsonObject run(int firstKind,int secondKind,int scenario,MinecraftServer s,ServerLevel level) {
        LivingEntity a=body(firstKind,s,level),b=body(secondKind,s,level);
        double[][] offsets={{0,0},{Math.nextDown((double).01f),0},{.01f,0},{Math.nextUp((double).01f),0},{.1,.1},{.4,.2},{1,0},{1,1},{4,0},{-4,0},{-.4,.25},{1e6,.25},{.25,-1e6},{-0.,-0.},{2,3}};
        double[] d=offsets[scenario<offsets.length?scenario:4];
        a.setPos(0,65,0);b.setPos(d[0],scenario==14?165:65,d[1]);
        a.setDeltaMovement(.125,-0.,.25);b.setDeltaMovement(-.5,.75,-.125);
        a.noPhysics=false;b.noPhysics=false;
        float pushthrough=0;
        level.setBlock(new net.minecraft.core.BlockPos(0,65,0),net.minecraft.world.level.block.Blocks.AIR.defaultBlockState(),0);
        if(scenario==15){a.setPose(Pose.SLEEPING);a.setSleepingPos(new net.minecraft.core.BlockPos(0,65,0));}
        if(scenario==16){b.setPose(Pose.SLEEPING);b.setSleepingPos(new net.minecraft.core.BlockPos(0,65,0));}
        if(scenario==17)a.noPhysics=true;
        if(scenario==18)b.noPhysics=true;
        if(scenario==19||scenario==21)riding(body(1,s,level),a);
        if(scenario==20||scenario==21)riding(body(1,s,level),b);
        if(scenario==22){LivingEntity root=body(2,s,level);riding(a,root);riding(b,root);}
        if(scenario==23)riding(b,a);
        if(scenario==24)a.setHealth(0);
        if(scenario==25)b.setHealth(0);
        if(scenario==26){a.setHealth(0);b.setHealth(0);}
        if(scenario==27)level.setBlock(new net.minecraft.core.BlockPos(0,65,0),net.minecraft.world.level.block.Blocks.LADDER.defaultBlockState(),0);
        if(scenario==28&&a instanceof ServerPlayer p)spectator(p);
        if(scenario==29&&b instanceof ServerPlayer p)spectator(p);
        if(scenario>=30)pushthrough=new float[]{.25f,.99999994f,1f,2f,-.5f}[scenario-30];
        
        a.needsSync=false;b.needsSync=false;
        JsonObject row=new JsonObject();row.addProperty("first_kind",firstKind);row.addProperty("second_kind",secondKind);row.addProperty("scenario",scenario);
        JsonObject input=new JsonObject();input.add("first",inputBody(a));input.add("second",inputBody(b));
        input.addProperty("first_sleeping",a.isSleeping());input.addProperty("same_vehicle",a.isPassengerOfSameVehicle(b));input.addProperty("legacy_pushthrough",(double)pushthrough);
        input.addProperty("pushthrough_bits",Integer.toUnsignedLong(Float.floatToRawIntBits(pushthrough)));row.add("input",input);
        a.push(b);
        row.add("first_velocity_bits",bits(velocity(a)));row.add("second_velocity_bits",bits(velocity(b)));
        row.addProperty("first_pushed",a.needsSync);row.addProperty("second_pushed",b.needsSync);
        level.setBlock(new net.minecraft.core.BlockPos(0,65,0),net.minecraft.world.level.block.Blocks.AIR.defaultBlockState(),0);
        return row;
    }
    public static void main(String[] args) {
        try {
            gameMode=net.minecraft.server.level.ServerPlayerGameMode.class.getDeclaredField(args[1]);gameMode.setAccessible(true);
            vehicle=Entity.class.getDeclaredField(args[2]);vehicle.setAccessible(true);
            passengers=Entity.class.getDeclaredField(args[3]);passengers.setAccessible(true);
            net.minecraft.server.Main.main(new String[]{"--nogui"});MinecraftServer server=null;
            for(int i=0;i<120&&(server==null||server.getTickCount()<20);i++){Thread.sleep(500);if(server==null)server=findServer();}
            if(server==null||server.getTickCount()<20)throw new IllegalStateException("original server not ready");
            MinecraftServer s=server;JsonArray rows=new JsonArray();
            s.executeBlocking(()->{for(int a=0;a<4;a++)for(int b=0;b<4;b++)for(int scenario=0;scenario<30;scenario++)rows.add(run(a,b,scenario,s,s.overworld()));});
            Files.writeString(Path.of(args[0]),new Gson().toJson(rows)+"\n");s.halt(false);System.exit(0);
        }catch(Throwable e){e.printStackTrace();System.exit(1);}
    }
}
