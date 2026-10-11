// Own headless harness: original packet handlers and ClientLevel dispatch; AI/travel and rendering isolated.
package voxrig.oracle;
import com.google.gson.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import java.util.concurrent.atomic.AtomicReference;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.entity.*;
import net.minecraft.world.entity.player.Player;
import com.mojang.authlib.GameProfile;
import net.minecraft.world.entity.animal.Cow;
import net.minecraft.client.multiplayer.*;
import net.minecraft.util.profiling.*;
import net.minecraft.world.level.Level;
import net.minecraft.client.Minecraft;
import net.minecraft.network.protocol.game.*;
import net.minecraft.world.level.chunk.*;
import it.unimi.dsi.fastutil.ints.*;

public final class EntityTickOracle {
    static List<JsonObject> seen;
    static Map<Integer,RemoteCow> byId;
    static Map<Integer,RemoteCow> all;
    static ClientLevel client;
    static ClientPacketListener listener;
    static int lifetime;
    static boolean mutation;
    static List<JsonObject> inside;
    static ServerLevel registered;
    static Int2ObjectOpenHashMap<Entity> legacyMap;
    static Field data,steps;
    static class ProbeChunkCache extends ClientChunkCache {
        ProbeChunkCache(ClientLevel l){super(l,4);}
        @Override public LevelChunk getChunk(int x,int z,ChunkStatus status,boolean create){return registered.getChunk(x,z);}
        @Override public boolean isEntityTickingChunk(Entity e){return ((RemoteCow)e).ticking;}
    }
    static class ProbePlayer extends Player {
        final int key;
        ProbePlayer(ServerLevel level,int id){super(level,new net.minecraft.core.BlockPos(0,65,0),new GameProfile(new UUID(0,id),"OraclePlayer"));key=++lifetime;setId(id);setPos(.1,65.2,-.3);inChunk=true;xChunk=0;yChunk=4;zChunk=-1;}
        @Override public boolean isSpectator(){return false;}
        @Override public boolean isCreative(){return false;}
        @Override public void tick(){JsonObject e=new JsonObject();e.addProperty("key",key);e.addProperty("native_id",getId());e.addProperty("dispatch","player_tick");e.add("before",pose(this));e.add("after",pose(this));seen.add(e);}
    }
    static class RemoteCow extends Cow {
        final int key;
        boolean ticking=true,local=false;
        RemoteCow(ServerLevel level,int id){super(EntityType.COW,level);key=++lifetime;setId(id);setPos(.1,65.2,-.3);setPacketCoordinates(.1,65.2,-.3);inChunk=true;xChunk=0;yChunk=4;zChunk=-1;}
        @Override public boolean isControlledByLocalInstance(){return local;}
        @Override public boolean isEffectiveAi(){return false;}
        @Override public void travel(net.minecraft.world.phys.Vec3 input){}
        int remaining(){ return lerpSteps; }
        void cancelModel(){ lerpSteps=0; }
        @Override public void tick(){
            JsonObject event=new JsonObject();event.addProperty("key",key);event.addProperty("native_id",getId());event.addProperty("dispatch","tick");event.add("before",pose(this));
            aiStep();
            event.add("after",pose(this));event.addProperty("remaining_steps",remaining());seen.add(event);
            
        }
        @Override public void rideTick(){JsonObject e=new JsonObject();e.addProperty("key",key);e.addProperty("native_id",getId());e.addProperty("dispatch","ride");seen.add(e);}
    }
    static Object allocate(Class<?> k) throws Exception {
        Class<?> u=Class.forName("sun.misc.Unsafe");Field f=u.getDeclaredField("theUnsafe");f.setAccessible(true);
        return u.getMethod("allocateInstance",Class.class).invoke(f.get(null),k);
    }
    static void set(Object o,Class<?> k,String name,Object v) throws Exception {Field f=k.getDeclaredField(name);f.setAccessible(true);f.set(o,v);}
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
        return pose(e.getX(),e.getY(),e.getZ(),e.yRot,e.xRot);
    }
    static JsonObject pose(double x,double y,double z,float yaw,float pitch) {
        JsonObject o=new JsonObject();JsonArray p=new JsonArray(),r=new JsonArray(),pb=new JsonArray(),rb=new JsonArray();
        for(double n:new double[]{x,y,z}){p.add(n);pb.add(Long.toUnsignedString(Double.doubleToRawLongBits(n),16));}
        for(float n:new float[]{yaw,pitch}){r.add((double)n);rb.add(Integer.toUnsignedLong(Float.floatToRawIntBits(n)));}
        o.add("position",p);o.add("rotation",r);o.add("position_bits",pb);o.add("rotation_bits",rb);return o;
    }
    static double n(JsonObject p,int i){return p.getAsJsonArray("position").get(i).getAsDouble();}
    static float r(JsonObject p,int i){return p.getAsJsonArray("rotation").get(i).getAsFloat();}
    static void spawn(int id,List<JsonObject> actions){RemoteCow e=new RemoteCow(registered,id);byId.put(id,e);all.put(e.key,e);legacyMap.put(id,e);
        JsonObject a=new JsonObject();a.addProperty("op","spawn");a.addProperty("native_id",id);a.addProperty("key",e.key);a.add("pose",pose(e));actions.add(a);
    }
    static JsonObject action(String op,int id){JsonObject a=new JsonObject();a.addProperty("op",op);a.addProperty("native_id",id);return a;}
    static void teleport(RemoteCow e,double distance,List<JsonObject> actions){
        double x=e.getX()+distance,y=e.getY(),z=e.getZ();float yaw=90,pitch=-45;
        JsonObject a=action("absolute",e.getId());a.add("position",new Gson().toJsonTree(new double[]{x,y,z}));a.add("rotation",new Gson().toJsonTree(new float[]{yaw,pitch}));a.addProperty("ground",true);actions.add(a);
        double ox=e.getX(),oy=e.getY(),oz=e.getZ();float or=e.yRot,op=e.xRot;boolean ground=e.isOnGround();
        e.setPos(x,y,z);e.yRot=yaw;e.xRot=pitch;e.setOnGround(true);ClientboundTeleportEntityPacket packet=new ClientboundTeleportEntityPacket(e);e.setPos(ox,oy,oz);e.yRot=or;e.xRot=op;e.setOnGround(ground);listener.handleTeleportEntity(packet);
    }
    static JsonObject evaluate(int seed,ServerLevel level,String[] args)throws Exception{
        lifetime=0;registered=level;byId=new LinkedHashMap<>();all=new LinkedHashMap<>();mutation=false;
        client=(ClientLevel)allocate(ClientLevel.class);
        for(Field f:Level.class.getDeclaredFields())if(!Modifier.isStatic(f.getModifiers())){f.setAccessible(true);f.set(client,f.get(level));}
        legacyMap=new Int2ObjectOpenHashMap<>();set(client,ClientLevel.class,args[1],legacyMap);set(client,ClientLevel.class,args[2],new ArrayList<>());set(client,ClientLevel.class,args[3],allocate(ProbeChunkCache.class));
        Minecraft minecraft=(Minecraft)allocate(Minecraft.class);
        set(minecraft,Minecraft.class,args[4],Thread.currentThread());
        listener=(ClientPacketListener)allocate(ClientPacketListener.class);
        set(listener,ClientPacketListener.class,args[5],minecraft);set(listener,ClientPacketListener.class,args[6],client);

        JsonArray out=new JsonArray();int count=new int[]{8,25,50,100}[seed%4];int primary=seed*257+1;
        for(int stage=0;stage<12;stage++){
            List<JsonObject> actions=new ArrayList<>();inside=new ArrayList<>();seen=new ArrayList<>();
            if(stage==0){for(int i=0;i<count;i++)spawn(i==0?0:(i==1?primary:(seed==0&&i<5?new int[]{0,0,33,755,66}[i]:(i%2==0?-1:1)*(seed*257+i*37))),actions);}ProbePlayer player=new ProbePlayer(level,2000000+seed);legacyMap.put(player.getId(),player);JsonObject p=action("spawn_player",player.getId());p.addProperty("key",player.key);p.add("pose",pose(player));actions.add(p);
            RemoteCow e=byId.get(primary);
            if(stage==1||stage==10){for(int i=0;i<2;i++){
                JsonObject a=action("relative",primary);a.add("delta",new Gson().toJsonTree(new int[]{512,-128,-512}));a.add("rotation",new Gson().toJsonTree(new int[]{64,-32}));a.addProperty("ground",true);actions.add(a);
                listener.handleMoveEntity(new ClientboundMoveEntityPacket.PosRot(primary,(short)512,(short)-128,(short)-512,(byte)64,(byte)-32,true));}}
            if(stage==2){JsonObject a=action("relative",primary);a.add("delta",new Gson().toJsonTree(new int[]{0,0,1}));a.addProperty("ground",false);actions.add(a);listener.handleMoveEntity(new ClientboundMoveEntityPacket.Pos(primary,(short)0,(short)0,(short)1,false));
                a=action("rotation",primary);a.add("rotation",new Gson().toJsonTree(new int[]{-64,32}));a.addProperty("ground",true);actions.add(a);listener.handleMoveEntity(new ClientboundMoveEntityPacket.Rot(primary,(byte)-64,(byte)32,true));}
            if(stage==3){teleport(e,seed%2==0?.001:.03125,actions);JsonObject a=action("cancel",primary);actions.add(a);e.cancelModel();}
            if(stage==4){JsonObject a=action("passenger",primary);a.addProperty("vehicle",0);actions.add(a);e.startRiding(byId.get(0),true);}
            if(stage==5){JsonObject a=action("local",primary);a.addProperty("value",true);actions.add(a);e.local=true;listener.handleMoveEntity(new ClientboundMoveEntityPacket.Pos(primary,(short)1,(short)0,(short)0,false));a=action("relative",primary);a.add("delta",new Gson().toJsonTree(new int[]{1,0,0}));a.addProperty("ground",false);actions.add(a);}
            if(stage==6){JsonObject a=action("unride",primary);actions.add(a);e.stopRiding();a.add("model_pose_after",pose(e));a=action("local",primary);a.addProperty("value",false);actions.add(a);e.local=false;}
            if(stage==6){RemoteCow retired=byId.get(0);retired.removed=true;retired.inChunk=false;actions.add(action("removed",0));if(seed==0){RemoteCow wrapped=byId.get(33);wrapped.removed=true;wrapped.inChunk=false;actions.add(action("removed",33));}}
            if(stage==7){for(int id:new ArrayList<>(byId.keySet()))if(id!=primary&&id!=0){RemoteCow old=byId.remove(id);legacyMap.remove(id);actions.add(action("remove_membership",id));}spawn(seed*997+1000,actions);}
            if(stage==8){e.ticking=false;actions.add(action("unticking",primary));}
            if(stage==9){e.ticking=true;actions.add(action("ticking",primary)); }
            if(stage==10){teleport(e,seed%2==0?64.:65.,actions);teleport(e,0.,actions);}
            if(stage==11){e.cancelModel();actions.add(action("cancel",primary));}
            client.tickEntities();
            JsonObject step=new JsonObject();step.addProperty("stage",stage);step.add("actions",new Gson().toJsonTree(actions));step.add("during_tick",new Gson().toJsonTree(inside));step.add("ticks",new Gson().toJsonTree(seen));JsonArray states=new JsonArray();
            for(RemoteCow body:byId.values()){JsonObject state=new JsonObject();state.addProperty("key",body.key);state.addProperty("native_id",body.getId());state.add("pose",pose(body));state.addProperty("remaining_steps",body.remaining());states.add(state);}step.add("states",states);out.add(step);
        }
        JsonObject row=new JsonObject();row.addProperty("seed",seed);row.add("steps",out);return row;
    }
    public static void main(String[] args){try {
        net.minecraft.server.Main.main(new String[]{"--nogui"});MinecraftServer server=null;
        for(int i=0;i<120&&(server==null||server.getTickCount()<20);i++){Thread.sleep(500);if(server==null)server=findServer();}
        if(server==null)throw new IllegalStateException("not ready");MinecraftServer s=server;
        JsonArray result=new JsonArray();s.executeBlocking(()->{try{ for(int seed=0;seed<16;seed++)result.add(evaluate(seed,s.overworld(),args)); }catch(Exception e){throw new RuntimeException(e);}});
        Files.writeString(Path.of(args[0]),new Gson().toJson(result)+"\n");s.halt(false);System.exit(0);
    }catch(Throwable e){e.printStackTrace();System.exit(1);}}
}
