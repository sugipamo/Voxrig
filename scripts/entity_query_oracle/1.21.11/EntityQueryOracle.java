// Own harness: original spatial query/list operations with explicitly staged model membership.
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
import net.minecraft.world.entity.animal.pig.Pig;
import net.minecraft.world.entity.monster.zombie.Zombie;
import net.minecraft.world.level.entity.*;
import net.minecraft.core.SectionPos;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.scores.*;
public final class EntityQueryOracle {
    static Field client;
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

    static class Index {
        final ServerLevel level;
        final IdentityHashMap<Entity,Long> keys=new IdentityHashMap<>();
        final Map<Long,Entity> entities=new HashMap<>();
        final Map<Long,String> kinds=new HashMap<>();
        final EntitySectionStorage<Entity> storage=new EntitySectionStorage<>(Entity.class, chunk->Visibility.TICKING);
        final Map<Long,Long> membership=new HashMap<>();
        Index(ServerLevel l){level=l;}
        JsonObject input(Entity e){
            JsonObject o=new JsonObject();o.addProperty("key",keys.get(e));o.addProperty("kind",kinds.get(keys.get(e)));
            o.add("body",body((LivingEntity)e));o.addProperty("spectator",e.isSpectator());
            o.addProperty("player",e instanceof ModelPlayer p?(p.local?"LocalPlayer":"RemotePlayer"):"NonPlayer");
            o.add("team_key",JsonNull.INSTANCE);o.add("vehicle_root",JsonNull.INSTANCE);return o;
        }
        void put(Entity e,long key,String kind,JsonArray actions){
            long next=SectionPos.asLong(net.minecraft.util.Mth.floor(e.getX())>>4,net.minecraft.util.Mth.floor(e.getY())>>4,net.minecraft.util.Mth.floor(e.getZ())>>4);
            Long old=membership.get(key);
            if(old==null || old.longValue()!=next) {
                if(old!=null){var section=storage.getSection(old);section.remove(e);if(section.isEmpty())storage.remove(old);}
                storage.getOrCreateSection(next).add(e);membership.put(key,next);
            }
            keys.put(e,key);entities.put(key,e);kinds.put(key,kind);
            JsonObject a=new JsonObject();a.addProperty("op","upsert");a.add("entity",input(e));actions.add(a);
        }
        void remove(long key,JsonArray actions){
            Entity e=entities.remove(key);if(e==null)throw new IllegalStateException("absent model key");
            long old=membership.remove(key);var section=storage.getSection(old);section.remove(e);if(section.isEmpty())storage.remove(old);
            JsonObject a=new JsonObject();a.addProperty("op","remove");a.addProperty("key",key);actions.add(a);
        }
        List<Entity> query(Entity q,AABB box){
            List<Entity> found=new ArrayList<>();storage.getEntities(box,e->{if(e!=q && EntitySelector.pushableBy(q).test(e))found.add(e);return net.minecraft.util.AbortableIterationConsumer.Continuation.CONTINUE;});return found;
        }
        void hidden(Entity e,JsonArray actions){
            long key=membership.get(keys.get(e));storage.getSection(key).updateChunkStatus(Visibility.HIDDEN);
            JsonObject a=new JsonObject();a.addProperty("op","visibility");JsonArray p=new JsonArray();p.add(SectionPos.x(key));p.add(SectionPos.y(key));p.add(SectionPos.z(key));a.add("section",p);a.addProperty("accessible",false);actions.add(a);
        }
    }
    static LivingEntity create(MinecraftServer s,ServerLevel l,int i){
        return switch(i%4){case 0->new ModelPlayer(s,l,(i/4)%2==0);case 1->new Cow(EntityType.COW,l);case 2->new Pig(EntityType.PIG,l);default->new Zombie(EntityType.ZOMBIE,l);};
    }
    static String kind(int i){return switch(i%4){case 0->"Player";case 1->"Cow";case 2->"Pig";default->"Zombie";};}
    static JsonObject scenario(MinecraftServer s,ServerLevel level,int seed,boolean clientSide)throws Exception {
        int base=switch(seed%4){case 0->-16;case 1->0;case 2->64;default->256;};
        Index index=new Index(level);LivingEntity q=new Cow(EntityType.COW,level);q.setPos(0,base+1,0);
        AABB box=new AABB(-18,base-4,-18,18,base+20,18);q.setBoundingBox(box);
        index.keys.put(q,9999L);index.kinds.put(9999L,"Cow");
        boolean previous=client.getBoolean(level);client.setBoolean(level,clientSide);
        try {
            JsonObject row=new JsonObject();row.addProperty("seed",seed);row.addProperty("client_side",clientSide);row.add("querying",index.input(q));
            JsonArray chunks=new JsonArray();for(int x=-2;x<=1;x++)for(int z=-2;z<=1;z++){JsonArray c=new JsonArray();c.add(x);c.add(z);chunks.add(c);}
            row.add("known_chunks",chunks);JsonArray steps=new JsonArray();LivingEntity[] bodies=new LivingEntity[16];long[] ids=new long[16];
            double[] axis={-16.,Math.nextDown(0.),0.,15.999,16.};double[] heights={-2.,-.001,0.,15.999,16.,31.999};
            for(int stage=0;stage<6;stage++){
                JsonArray actions=new JsonArray();
                if(stage==0){
                    for(int i=0;i<16;i++){
                        LivingEntity e=create(s,level,i);ids[i]=100L+(i*11)%17;bodies[i]=e;
                        e.setPos(axis[(i+seed)%axis.length],base+heights[(i*3+seed)%heights.length],axis[(i*2+seed)%axis.length]);
                        if((i+seed)%13==0)e.setHealth(0);
                        index.put(e,ids[i],kind(i),actions);
                    }
                    index.put(q,9999L,"Cow",actions);
                }
                if(stage==1){LivingEntity e=bodies[0];e.setPos(e.getX()+.125,e.getY()+.001,e.getZ()+.125);index.put(e,ids[0],kind(0),actions);}
                if(stage==2){LivingEntity e=bodies[1];e.setPos(0,base+16,0);index.put(e,ids[1],kind(1),actions);}
                if(stage==3){index.remove(ids[3],actions);index.put(bodies[3],ids[3],kind(3),actions);}
                if(stage==4){
                    LivingEntity old=bodies[7];index.remove(ids[7],actions);LivingEntity replacement=create(s,level,7);
                    replacement.setPos(old.getX(),old.getY(),old.getZ());ids[7]=1007;bodies[7]=replacement;index.put(replacement,ids[7],kind(7),actions);
                }
                if(stage==5)index.hidden(bodies[4],actions);
                JsonArray found=new JsonArray();for(Entity e:index.query(q,box)){
                    Long key=index.keys.get(e);if(key==null)throw new IllegalStateException("unexpected world entity");found.add(key);
                }
                JsonObject step=new JsonObject();step.add("actions",actions);step.add("keys",found);steps.add(step);
            }
            row.add("steps",steps);JsonArray cleanup=new JsonArray();for(long key:new ArrayList<>(index.entities.keySet()))index.remove(key,cleanup);
            return row;
        }finally{client.setBoolean(level,previous);}
    }
    public static void main(String[] args){try{
        client=Level.class.getDeclaredField(args[1]);client.setAccessible(true);
        net.minecraft.server.Main.main(new String[]{"--nogui"});MinecraftServer server=null;
        for(int i=0;i<120&&(server==null||server.getTickCount()<20);i++){Thread.sleep(500);if(server==null)server=findServer();}
        if(server==null||server.getTickCount()<20)throw new IllegalStateException("original server not ready");MinecraftServer s=server;JsonArray rows=new JsonArray();
        s.executeBlocking(()->{try{for(boolean side:new boolean[]{false,true})for(int seed=0;seed<24;seed++)rows.add(scenario(s,s.overworld(),seed,side));}catch(Exception e){throw new RuntimeException(e);}});
        Files.writeString(Path.of(args[0]),new GsonBuilder().serializeNulls().create().toJson(rows)+"\n");s.halt(false);System.exit(0);
    }catch(Throwable e){e.printStackTrace();System.exit(1);}}
}
