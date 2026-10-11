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
import net.minecraft.world.entity.animal.cow.Cow;
import net.minecraft.client.multiplayer.*;
import net.minecraft.util.profiling.*;
import net.minecraft.world.TickRateManager;
import net.minecraft.world.level.entity.EntityTickList;
import net.minecraft.world.level.Level;
import net.minecraft.client.Minecraft;
import net.minecraft.network.PacketProcessor;
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
    static EntityTickList modernList;
    static TickRateManager rates;
    static Field data,steps;
    static class ProbeClientLevel extends ClientLevel {
        ProbeClientLevel(){super(null,null,null,null,4,4,null,false,0,63);}
        @Override public Entity getEntity(int id){return byId.get(id);}
    }
    static class ProbePlayer extends Player {
        final int key;
        ProbePlayer(ServerLevel level,int id){super(level,new GameProfile(new UUID(0,id),"OraclePlayer"));key=++lifetime;setId(id);setPos(.1,65.2,-.3);}
        @Override public net.minecraft.world.level.GameType gameMode(){return net.minecraft.world.level.GameType.SURVIVAL;}
        @Override public void tick(){JsonObject e=new JsonObject();e.addProperty("key",key);e.addProperty("native_id",getId());e.addProperty("dispatch","player_tick");e.add("before",pose(this));e.add("after",pose(this));seen.add(e);}
    }
    static class RemoteCow extends Cow {
        final int key;
        boolean ticking=true,local=false;
        RemoteCow(ServerLevel level,int id){super(EntityType.COW,level);key=++lifetime;setId(id);setPos(.1,65.2,-.3);getPositionCodec().setBase(position());}
        @Override protected boolean isLocalClientAuthoritative(){return local;}
        int remaining(){ try{return steps.getInt(data.get(getInterpolation()));}catch(Exception e){throw new RuntimeException(e);} }
        void cancelModel(){ getInterpolation().cancel(); }
        @Override public void tick(){
            JsonObject event=new JsonObject();event.addProperty("key",key);event.addProperty("native_id",getId());event.addProperty("dispatch","tick");event.add("before",pose(this));
            getInterpolation().interpolate();
            event.add("after",pose(this));event.addProperty("remaining_steps",remaining());seen.add(event);
            if(mutation){mutation=false;int victim=byId.keySet().stream().filter(i->i!=getId()&&!byId.get(i).isRemoved()).findFirst().orElseThrow();
                RemoteCow retired=byId.remove(victim);modernList.remove(retired);
                JsonObject action=new JsonObject();action.addProperty("op","remove_membership");action.addProperty("native_id",victim);inside.add(action);
                spawn(victim,inside);}

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
    static void spawn(int id,List<JsonObject> actions){RemoteCow e=new RemoteCow(registered,id);byId.put(id,e);all.put(e.key,e);modernList.add(e);
        JsonObject a=new JsonObject();a.addProperty("op","spawn");a.addProperty("native_id",id);a.addProperty("key",e.key);a.add("pose",pose(e));actions.add(a);
    }
    static JsonObject action(String op,int id){JsonObject a=new JsonObject();a.addProperty("op",op);a.addProperty("native_id",id);return a;}
    static void teleport(RemoteCow e,double distance,List<JsonObject> actions){
        double x=e.getX()+distance,y=e.getY(),z=e.getZ();float yaw=90,pitch=-45;
        JsonObject a=action("absolute",e.getId());a.add("position",new Gson().toJsonTree(new double[]{x,y,z}));a.add("rotation",new Gson().toJsonTree(new float[]{yaw,pitch}));a.addProperty("ground",true);actions.add(a);
        a.add("velocity",new Gson().toJsonTree(new double[]{.125,-.25,.375}));
        listener.handleEntityPositionSync(new ClientboundEntityPositionSyncPacket(e.getId(),new PositionMoveRotation(new net.minecraft.world.phys.Vec3(x,y,z),new net.minecraft.world.phys.Vec3(.125,-.25,.375),yaw,pitch),true));
    }
    static JsonObject evaluate(int seed,ServerLevel level,String[] args)throws Exception{
        lifetime=0;registered=level;byId=new LinkedHashMap<>();all=new LinkedHashMap<>();mutation=false;
        client=(ClientLevel)allocate(ProbeClientLevel.class);
        for(Field f:Level.class.getDeclaredFields())if(!Modifier.isStatic(f.getModifiers())){f.setAccessible(true);f.set(client,f.get(level));}
        modernList=new EntityTickList();rates=new TickRateManager();set(client,ClientLevel.class,args[1],modernList);set(client,ClientLevel.class,args[2],rates);
        Minecraft minecraft=(Minecraft)allocate(Minecraft.class);
        set(minecraft,Minecraft.class,args[3],new PacketProcessor(Thread.currentThread()));
        listener=(ClientPacketListener)allocate(ClientPacketListener.class);
        set(listener,ClientCommonPacketListenerImpl.class,args[4],minecraft);set(listener,ClientPacketListener.class,args[5],client);

        JsonArray out=new JsonArray();int count=new int[]{8,25,50,100}[seed%4];int primary=seed*257+1;
        for(int stage=0;stage<12;stage++){
            List<JsonObject> actions=new ArrayList<>();inside=new ArrayList<>();seen=new ArrayList<>();
            if(stage==0){for(int i=0;i<count;i++)spawn(i==0?0:(i==1?primary:(i%2==0?-1:1)*(seed*257+i*37)),actions);}ProbePlayer player=new ProbePlayer(level,2000000+seed);modernList.add(player);JsonObject p=action("spawn_player",player.getId());p.addProperty("key",player.key);p.add("pose",pose(player));actions.add(p);
            RemoteCow e=byId.get(primary);
            if(stage==1||stage==10){for(int i=0;i<2;i++){
                JsonObject a=action("relative",primary);a.add("delta",new Gson().toJsonTree(new int[]{512,-128,-512}));a.add("rotation",new Gson().toJsonTree(new int[]{64,-32}));a.addProperty("ground",true);actions.add(a);
                listener.handleMoveEntity(new ClientboundMoveEntityPacket.PosRot(primary,(short)512,(short)-128,(short)-512,(byte)64,(byte)-32,true));}}
            if(stage==2){JsonObject a=action("relative",primary);a.add("delta",new Gson().toJsonTree(new int[]{0,0,1}));a.addProperty("ground",false);actions.add(a);listener.handleMoveEntity(new ClientboundMoveEntityPacket.Pos(primary,(short)0,(short)0,(short)1,false));
                a=action("rotation",primary);a.add("rotation",new Gson().toJsonTree(new int[]{-64,32}));a.addProperty("ground",true);actions.add(a);listener.handleMoveEntity(new ClientboundMoveEntityPacket.Rot(primary,(byte)-64,(byte)32,true));}
            if(stage==3){teleport(e,seed%2==0?.001:.03125,actions);JsonObject a=action("cancel",primary);actions.add(a);e.cancelModel();}
            if(stage==4){JsonObject a=action("passenger",primary);a.addProperty("vehicle",0);actions.add(a);e.startRiding(byId.get(0),true,false);}
            if(stage==5){JsonObject a=action("local",primary);a.addProperty("value",true);actions.add(a);e.local=true;listener.handleMoveEntity(new ClientboundMoveEntityPacket.Pos(primary,(short)1,(short)0,(short)0,false));a=action("relative",primary);a.add("delta",new Gson().toJsonTree(new int[]{1,0,0}));a.addProperty("ground",false);actions.add(a);}
            if(stage==6){JsonObject a=action("unride",primary);actions.add(a);e.stopRiding();a.add("model_pose_after",pose(e));a=action("local",primary);a.addProperty("value",false);actions.add(a);e.local=false;}
            if(stage==6){RemoteCow retired=byId.get(0);retired.remove(Entity.RemovalReason.DISCARDED);actions.add(action("removed",0));}
            if(stage==7){for(int id:new ArrayList<>(byId.keySet()))if(id!=primary&&id!=0){RemoteCow old=byId.remove(id);modernList.remove(old);actions.add(action("remove_membership",id));}spawn(seed*997+1000,actions);}
            if(stage==8){rates.setFrozen(true);rates.tick();actions.add(action("freeze",primary));}
            if(stage==9){rates.setFrozen(false);rates.tick();actions.add(action("unfreeze",primary)); mutation=true;}
            if(stage==10){teleport(e,seed%2==0?64.:65.,actions);teleport(e,0.,actions);}
            if(stage==11){e.cancelModel();actions.add(action("cancel",primary));}
            client.tickEntities();
            JsonObject step=new JsonObject();step.addProperty("stage",stage);step.add("actions",new Gson().toJsonTree(actions));step.add("during_tick",new Gson().toJsonTree(inside));step.add("ticks",new Gson().toJsonTree(seen));JsonArray states=new JsonArray();
            for(RemoteCow body:byId.values()){JsonObject state=new JsonObject();state.addProperty("key",body.key);state.addProperty("native_id",body.getId());state.add("pose",pose(body));state.addProperty("remaining_steps",body.remaining());states.add(state);}step.add("states",states);out.add(step);
        }
        JsonObject row=new JsonObject();row.addProperty("seed",seed);row.add("steps",out);return row;
    }
    public static void main(String[] args){try {
        data=InterpolationHandler.class.getDeclaredField(args[7]);data.setAccessible(true);steps=Class.forName(InterpolationHandler.class.getName()+"$"+args[9]).getDeclaredField(args[8]);steps.setAccessible(true);
        net.minecraft.server.Main.main(new String[]{"--nogui"});MinecraftServer server=null;
        for(int i=0;i<120&&(server==null||server.getTickCount()<20);i++){Thread.sleep(500);if(server==null)server=findServer();}
        if(server==null)throw new IllegalStateException("not ready");MinecraftServer s=server;
        JsonArray result=new JsonArray();s.executeBlocking(()->{try{set(s.overworld(),Level.class,args[6],true);try{for(int seed=0;seed<16;seed++)result.add(evaluate(seed,s.overworld(),args));}finally{set(s.overworld(),Level.class,args[6],false);}}catch(Exception e){throw new RuntimeException(e);}});
        Files.writeString(Path.of(args[0]),new Gson().toJson(result)+"\n");s.halt(false);System.exit(0);
    }catch(Throwable e){e.printStackTrace();System.exit(1);}}
}
