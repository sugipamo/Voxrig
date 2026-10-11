// Own harness: unchanged selector/AABB/doPush calls; explicit candidate order.
package voxrig.oracle;
import com.google.gson.*;
import com.mojang.authlib.GameProfile;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import java.util.concurrent.atomic.AtomicReference;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.*;
import net.minecraft.world.entity.*;
import net.minecraft.world.entity.animal.cow.Cow;
import net.minecraft.world.level.Level;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.scores.*;
public final class EntityContactOracle {
    static Field client,gameMode,vehicle,passengers;
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
        }return null;
    }
    static class QueryCow extends Cow {
        QueryCow(ServerLevel l){super(EntityType.COW,l);}
        void dispatch(Entity e){super.doPush(e);}
    }
    static class ModelPlayer extends ServerPlayer {
        final boolean local;
        ModelPlayer(MinecraftServer s,ServerLevel l,boolean local){super(s,l,new GameProfile(UUID.randomUUID(),"Oracle"+UUID.randomUUID().toString().substring(0,8)),ClientInformation.createDefault());this.local=local;}
        @Override public boolean isLocalPlayer(){return local;}
    }
    static JsonArray bits(double... v){JsonArray a=new JsonArray();for(double n:v)a.add(Long.toUnsignedString(Double.doubleToRawLongBits(n),16));return a;}
    static JsonArray vec(double... v){JsonArray a=new JsonArray();for(double n:v)a.add(n);return a;}
    static JsonArray velocity(Entity e){var v=e.getDeltaMovement();return bits(v.x,v.y,v.z);}
    static String rule(Team t){return t==null?null:switch(t.getCollisionRule().name){case "pushOtherTeams"->"push_other_teams";case "pushOwnTeam"->"push_own_team";default->t.getCollisionRule().name;};}
    static JsonObject body(LivingEntity e){
        JsonObject o=new JsonObject(),p=new JsonObject(),b=new JsonObject();var box=e.getBoundingBox();var v=e.getDeltaMovement();
        p.add("position",vec(e.getX(),e.getY(),e.getZ()));p.add("position_bits",bits(e.getX(),e.getY(),e.getZ()));p.add("velocity",vec(v.x,v.y,v.z));p.add("velocity_bits",velocity(e));
        p.addProperty("no_physics",e.noPhysics);p.addProperty("vehicle",e.isVehicle());p.addProperty("pushable",e.isPushable());o.add("push",p);
        b.addProperty("min_x",box.minX);b.addProperty("min_y",box.minY);b.addProperty("min_z",box.minZ);b.addProperty("max_x",box.maxX);b.addProperty("max_y",box.maxY);b.addProperty("max_z",box.maxZ);o.add("bounds",b);
        o.add("bounds_bits",bits(box.minX,box.minY,box.minZ,box.maxX,box.maxY,box.maxZ));
        o.addProperty("sleeping",e.isSleeping());o.addProperty("legacy_pushthrough",0.);o.addProperty("team",rule(e.getTeam()));return o;
    }
    static void riding(Entity e,Entity root)throws Exception {vehicle.set(e,root);passengers.set(root,com.google.common.collect.ImmutableList.of(e));}
    static void setupTeam(ServerLevel level,Entity e,String name,int r){if(r<0)return;var scoreboard=level.getScoreboard();var t=scoreboard.getPlayerTeam(name);if(t==null)t=scoreboard.addPlayerTeam(name);t.setCollisionRule(Team.CollisionRule.values()[r]);scoreboard.addPlayerToTeam(e.getScoreboardName(),t);}
    static JsonObject run(MinecraftServer s,ServerLevel level,int qr,int cr,boolean allied,boolean clientSide,int kind,int scenario)throws Exception {
        QueryCow q=new QueryCow(level);LivingEntity c=kind==2?new Cow(EntityType.COW,level):new ModelPlayer(s,level,kind==0);LivingEntity d=new Cow(EntityType.COW,level);
        q.setPos(0,65,0);c.setPos(.4,65,.2);d.setPos(-.3,65,.1);
        q.setDeltaMovement(.125,-0.,.25);c.setDeltaMovement(-.5,.75,-.125);d.setDeltaMovement(.5,-0.,-.25);
        setupTeam(level,q,"query",qr);setupTeam(level,c,allied?"query":"candidate",cr);
        level.setBlock(new net.minecraft.core.BlockPos(0,65,0),net.minecraft.world.level.block.Blocks.AIR.defaultBlockState(),0);
        if(scenario==10){q.setPose(Pose.SLEEPING);q.setSleepingPos(new net.minecraft.core.BlockPos(0,65,0));}
        if(scenario==11){c.setPose(Pose.SLEEPING);c.setSleepingPos(new net.minecraft.core.BlockPos(0,65,0));}
        if(scenario==12)q.noPhysics=true;
        if(scenario==13)c.noPhysics=true;
        if(scenario==14)c.setHealth(0);
        if(scenario==15&&c instanceof ServerPlayer p)gameMode.set(p.gameMode,net.minecraft.world.level.GameType.SPECTATOR);
        if(scenario==16)level.setBlock(new net.minecraft.core.BlockPos(0,65,0),net.minecraft.world.level.block.Blocks.LADDER.defaultBlockState(),0);
        if(scenario==17)riding(c,q);
        if(scenario==18)q.setHealth(0);
        if(scenario==19){}
        AABB qb=new AABB(-.5,65,-.5,.5,66,.5),cb=new AABB(-.25,65,-.25,.75,66,.75);
        double edge=scenario==7?Math.nextDown(.5):scenario==8?Math.nextUp(.5):.5;
        if(scenario==1||scenario==7||scenario==8)cb=new AABB(edge,65,-.25,1.5,66,.75);
        if(scenario==2)cb=new AABB(-1.5,65,-.25,-.5,66,.75);
        if(scenario==3)cb=new AABB(-.25,66,-.25,.75,67,.75);
        if(scenario==4)cb=new AABB(-.25,64,-.25,.75,65,.75);
        if(scenario==5)cb=new AABB(-.25,65,.5,.75,66,1.5);
        if(scenario==6)cb=new AABB(-.25,65,-1.5,.75,66,-.5);
        if(scenario==9)cb=new AABB(0,65,0,0,66,0);
        q.setBoundingBox(qb);c.setBoundingBox(cb);d.setBoundingBox(new AABB(-.75,65,-.25,.25,66,.75));
        LivingEntity[] ordered=scenario==20?new LivingEntity[]{d,c}:scenario==21?new LivingEntity[]{q,c}:new LivingEntity[]{c,d};
        boolean previous=client.getBoolean(level);client.setBoolean(level,clientSide);
        try{
            JsonObject row=new JsonObject();row.addProperty("query_rule",qr);row.addProperty("candidate_rule",cr);row.addProperty("allied",allied);row.addProperty("client_side",clientSide);row.addProperty("kind",kind);row.addProperty("scenario",scenario);row.add("querying",body(q));
            JsonArray candidates=new JsonArray(),results=new JsonArray();
            for(LivingEntity e:ordered){JsonObject input=new JsonObject();input.add("body",body(e));input.addProperty("same_body",e==q);input.addProperty("spectator",e.isSpectator());input.addProperty("player",e instanceof ModelPlayer p?(p.local?"LocalPlayer":"RemotePlayer"):"NonPlayer");input.addProperty("allied",q.getTeam()!=null&&q.getTeam().isAlliedTo(e.getTeam()));input.addProperty("same_vehicle",e.isPassengerOfSameVehicle(q));candidates.add(input);}
            row.add("candidates",candidates);
            for(LivingEntity e:ordered){boolean admitted=e!=q&&qb.intersects(e.getBoundingBox())&&EntitySelector.pushableBy(q).test(e);q.needsSync=false;e.needsSync=false;if(admitted)q.dispatch(e);
                JsonObject output=new JsonObject();output.addProperty("admitted",admitted);output.add("querying_velocity_bits",velocity(q));output.add("candidate_velocity_bits",velocity(e));output.addProperty("querying_pushed",q.needsSync);output.addProperty("candidate_pushed",e.needsSync);results.add(output);}
            row.add("results",results);return row;
        }finally{client.setBoolean(level,previous);var scoreboard=level.getScoreboard();for(String name:new String[]{"query","candidate"}){var t=scoreboard.getPlayerTeam(name);if(t!=null)scoreboard.removePlayerTeam(t);}level.setBlock(new net.minecraft.core.BlockPos(0,65,0),net.minecraft.world.level.block.Blocks.AIR.defaultBlockState(),0);}
    }
    public static void main(String[] args){try{
        client=Level.class.getDeclaredField(args[1]);client.setAccessible(true);gameMode=ServerPlayerGameMode.class.getDeclaredField(args[2]);gameMode.setAccessible(true);vehicle=Entity.class.getDeclaredField(args[3]);vehicle.setAccessible(true);passengers=Entity.class.getDeclaredField(args[4]);passengers.setAccessible(true);
        net.minecraft.server.Main.main(new String[]{"--nogui"});MinecraftServer server=null;
        for(int i=0;i<120&&(server==null||server.getTickCount()<20);i++){Thread.sleep(500);if(server==null)server=findServer();}
        if(server==null||server.getTickCount()<20)throw new IllegalStateException("original server not ready");MinecraftServer s=server;JsonArray rows=new JsonArray();
        s.executeBlocking(()->{try{for(boolean local:new boolean[]{false,true})for(int kind=0;kind<3;kind++){
            for(int qr=-1;qr<4;qr++)for(int cr=-1;cr<4;cr++)rows.add(run(s,s.overworld(),qr,cr,false,local,kind,0));
            for(int r=0;r<4;r++)rows.add(run(s,s.overworld(),r,r,true,local,kind,0));
            for(int scenario=1;scenario<=21;scenario++)rows.add(run(s,s.overworld(),-1,-1,false,local,kind,scenario));
        }}catch(Exception e){throw new RuntimeException(e);}});
        Files.writeString(Path.of(args[0]),new GsonBuilder().serializeNulls().create().toJson(rows)+"\n");s.halt(false);System.exit(0);
    }catch(Throwable e){e.printStackTrace();System.exit(1);}}
}
