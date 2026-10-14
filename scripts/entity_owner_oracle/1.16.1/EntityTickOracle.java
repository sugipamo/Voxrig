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
    static Map<Integer,LivingEntity> byId;
    static Map<Integer,LivingEntity> all;
    static ClientLevel client;
    static ClientPacketListener listener;
    static int lifetime;
    static Map<LivingEntity,Integer> keys;
    static Set<Integer> unticking;
    static int actorKind;
    static boolean boatMode;
    static ProbeBoat controlledBoat;
    static List<Entity> admitted;
    static boolean mutation;
    static List<JsonObject> inside;
    static ServerLevel registered;
    static Int2ObjectOpenHashMap<Entity> legacyMap;
    static Field data,steps;
    static class ProbeClientLevel extends ClientLevel {
        ProbeClientLevel(){super(null,null,null,null,null,4,null,null,false,0);}
        // Audio delivery only; the original movement/step callback still runs.
        @Override public void sendPacketToServer(net.minecraft.network.protocol.Packet<?> packet){}
        @Override public void playLocalSound(double x,double y,double z,net.minecraft.sounds.SoundEvent sound,net.minecraft.sounds.SoundSource category,float volume,float pitch,boolean distance){}
    }
    static class ProbeChunkCache extends ClientChunkCache {
        ProbeChunkCache(ClientLevel l){super(l,4);}
        @Override public LevelChunk getChunk(int x,int z,ChunkStatus status,boolean create){return registered.getChunk(x,z);}
        @Override public net.minecraft.world.level.lighting.LevelLightEngine getLightEngine(){return registered.getChunkSource().getLightEngine();}
        @Override public boolean isEntityTickingChunk(Entity e){return !unticking.contains(e.getId());}
    }
    static class ProbePlayer extends Player {
        final int key;
        ProbePlayer(ServerLevel level,int id){super(level,new net.minecraft.core.BlockPos(0,65,0),new GameProfile(new UUID(0,id),"OraclePlayer"));key=++lifetime;setId(id);setPos(.4,65.0,-.3);setDeltaMovement(.02,.0025,-.035);yRot=0;xRot=0;inChunk=true;xChunk=0;yChunk=4;zChunk=-1;}
        @Override public boolean isSpectator(){return false;}
        @Override public boolean isCreative(){return false;}
        @Override public boolean isLocalPlayer(){return true;}
        @Override public void tick(){JsonObject e=new JsonObject();e.addProperty("key",key);e.addProperty("native_id",getId());e.addProperty("dispatch","player_tick");e.add("before",pose(this));super.aiStep();e.add("after",pose(this));seen.add(e);}
    }
    static java.lang.invoke.MethodHandle originalPlayerAiStep;
    static class ProbeLocalPlayer extends net.minecraft.client.player.LocalPlayer {
        int key;
        ProbeLocalPlayer(){super(null,null,null,null,null,false,false);}
        @Override public boolean isSpectator(){return false;}
        @Override public boolean isCreative(){return false;}
        @Override public void rideTick(){}
        @Override public void tick(){JsonObject e=new JsonObject();e.addProperty("key",key);e.addProperty("native_id",getId());e.addProperty("dispatch","player_tick");e.add("before",pose(this));try{originalPlayerAiStep.invokeExact((Player)this);}catch(Throwable error){throw new RuntimeException(error);}e.add("after",pose(this));seen.add(e);}
    }
    static ProbeLocalPlayer local(ProbePlayer seed,Minecraft minecraft,String[] args)throws Exception{
        ProbeLocalPlayer body=(ProbeLocalPlayer)allocate(ProbeLocalPlayer.class);
        for(Class<?> k=Player.class;k!=Object.class;k=k.getSuperclass())for(Field f:k.getDeclaredFields())if(!Modifier.isStatic(f.getModifiers())){f.setAccessible(true);f.set(body,f.get(seed));}
        body.key=seed.key;set(body.getEntityData(),net.minecraft.network.syncher.SynchedEntityData.class,args[16],body);body.input=new net.minecraft.client.player.Input();body.level=client;set(body,net.minecraft.client.player.LocalPlayer.class,args[11],minecraft);set(body,net.minecraft.client.player.LocalPlayer.class,args[12],listener);set(minecraft,Minecraft.class,args[13],body);return body;
    }
    static class ProbeBoat extends net.minecraft.world.entity.vehicle.Boat {
        int key;
        ProbeBoat(){super(client,.4,65.,-.3);key=++lifetime;setId(3000000);setPos(.4,65.,-.3);yRot=0;xRot=0;setDeltaMovement(.02,0.,.08);}
        @Override public void tick(){JsonObject e=new JsonObject();e.addProperty("key",key);e.addProperty("native_id",getId());e.addProperty("dispatch","boat_tick");e.add("before",pose(this));net.minecraft.world.phys.AABB box=getBoundingBox();box=new net.minecraft.world.phys.AABB(box.minX-1.0000001,box.minY-5.0000001,box.minZ-1.0000001,box.maxX+1.0000001,box.maxY+5.0000001,box.maxZ+1.0000001);JsonArray candidates=new JsonArray();for(Entity other:client.getEntities(this,box,EntitySelector.NO_SPECTATORS)){if(other instanceof LivingEntity living && keys.containsKey(living) && other.isPushable())candidates.add(key(living));}e.add("geometry_query",candidates);super.tick();e.add("after",pose(this));seen.add(e);}
    }
    static class ContactCat extends net.minecraft.world.entity.animal.Cat {
        ContactCat(){super(EntityType.CAT,client);}
        @Override public void tick(){dispatch(this,()->super.aiStep());}
    }
    static class ContactDolphin extends net.minecraft.world.entity.animal.Dolphin {
        ContactDolphin(){super(EntityType.DOLPHIN,client);}
        @Override public void tick(){dispatch(this,()->super.aiStep());}
    }
    static class ContactGiant extends net.minecraft.world.entity.monster.Giant {
        ContactGiant(){super(EntityType.GIANT,client);}
        @Override public void tick(){dispatch(this,()->super.aiStep());}
    }
    static class ContactMooshroom extends net.minecraft.world.entity.animal.MushroomCow {
        ContactMooshroom(){super(EntityType.MOOSHROOM,client);}
        @Override public void tick(){dispatch(this,()->super.aiStep());}
    }
    static class ContactOcelot extends net.minecraft.world.entity.animal.Ocelot {
        ContactOcelot(){super(EntityType.OCELOT,client);}
        @Override public void tick(){dispatch(this,()->super.aiStep());}
    }
    static class ContactPanda extends net.minecraft.world.entity.animal.Panda {
        ContactPanda(){super(EntityType.PANDA,client);}
        @Override public void tick(){dispatch(this,()->super.aiStep());}
    }
    static class ContactSilverfish extends net.minecraft.world.entity.monster.Silverfish {
        ContactSilverfish(){super(EntityType.SILVERFISH,client);}
        @Override public void tick(){dispatch(this,()->super.aiStep());}
    }
    static class ContactVillager extends net.minecraft.world.entity.npc.Villager {
        ContactVillager(){super(EntityType.VILLAGER,client);}
        @Override public void tick(){dispatch(this,()->super.aiStep());}
    }
    static class ContactHorse extends net.minecraft.world.entity.animal.horse.Horse {
        ContactHorse(){super(EntityType.HORSE,client);}
        @Override public void tick(){dispatch(this,()->super.aiStep());}
    }
    static class RemoteSheep extends net.minecraft.world.entity.animal.Sheep {
        RemoteSheep(){super(EntityType.SHEEP,client);}
        @Override public void tick(){dispatch(this,()->super.aiStep());}
    }
    static class RemoteCreeper extends net.minecraft.world.entity.monster.Creeper {
        RemoteCreeper(){super(EntityType.CREEPER,client);}
        @Override public void tick(){dispatch(this,()->super.aiStep());}
    }
    static class RemoteCow extends Cow {
        RemoteCow(){super(EntityType.COW,client);}
        @Override public void tick(){dispatch(this,()->super.aiStep());}
    }
    static class RemotePig extends net.minecraft.world.entity.animal.Pig {
        RemotePig(){super(EntityType.PIG,client);}
        @Override public void tick(){dispatch(this,()->super.aiStep());}
    }
    static class RemoteZombie extends net.minecraft.world.entity.monster.Zombie {
        RemoteZombie(){super(EntityType.ZOMBIE,client);}
        @Override public void tick(){dispatch(this,()->super.aiStep());}
    }
    static class RemoteActorPlayer extends net.minecraft.client.player.RemotePlayer {
        RemoteActorPlayer(int id){super(client,new GameProfile(new UUID(1,id),"Remote"+id));}
        @Override public boolean isSpectator(){return false;}
        @Override public boolean isCreative(){return false;}
        @Override public void tick(){dispatch(this,()->super.aiStep());}
    }
    static int key(LivingEntity e){return keys.get(e);}
    static int remaining(LivingEntity e){try{return steps.getInt(e);}catch(Exception error){throw new RuntimeException(error);}}
    static void cancelModel(LivingEntity e){try{steps.setInt(e,0);}catch(Exception error){throw new RuntimeException(error);}}
    static String name(LivingEntity e){return e instanceof ContactCat?"minecraft:cat":e instanceof ContactDolphin?"minecraft:dolphin":e instanceof ContactGiant?"minecraft:giant":e instanceof ContactMooshroom?"minecraft:mooshroom":e instanceof ContactOcelot?"minecraft:ocelot":e instanceof ContactPanda?"minecraft:panda":e instanceof ContactSilverfish?"minecraft:silverfish":e instanceof ContactVillager?"minecraft:villager":e instanceof ContactHorse?"minecraft:horse":e instanceof RemoteSheep?"minecraft:sheep":e instanceof RemoteCreeper?"minecraft:creeper":e instanceof RemoteActorPlayer?"minecraft:player":e instanceof RemotePig?"minecraft:pig":e instanceof RemoteZombie?"minecraft:zombie":"minecraft:cow";}
    static void dispatch(LivingEntity e,Runnable aiStep){
        JsonObject event=new JsonObject();event.addProperty("key",key(e));event.addProperty("native_id",e.getId());event.addProperty("dispatch","tick");event.add("before",pose(e));
        aiStep.run();
        event.add("after",pose(e));event.addProperty("remaining_steps",remaining(e));seen.add(event);
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
        JsonObject p=pose(e.getX(),e.getY(),e.getZ(),e.yRot,e.xRot);JsonArray velocity=new JsonArray(),bits=new JsonArray();for(double n:new double[]{e.getDeltaMovement().x,e.getDeltaMovement().y,e.getDeltaMovement().z}){velocity.add(n);bits.add(Long.toUnsignedString(Double.doubleToRawLongBits(n),16));}p.add("velocity",velocity);p.add("velocity_bits",bits);net.minecraft.world.phys.AABB b=e.getBoundingBox();p.add("bounds",new Gson().toJsonTree(new double[]{b.minX,b.minY,b.minZ,b.maxX,b.maxY,b.maxZ}));return p;
    }
    static JsonObject pose(double x,double y,double z,float yaw,float pitch) {
        JsonObject o=new JsonObject();JsonArray p=new JsonArray(),r=new JsonArray(),pb=new JsonArray(),rb=new JsonArray();
        for(double n:new double[]{x,y,z}){p.add(n);pb.add(Long.toUnsignedString(Double.doubleToRawLongBits(n),16));}
        for(float n:new float[]{yaw,pitch}){r.add((double)n);rb.add(Integer.toUnsignedLong(Float.floatToRawIntBits(n)));}
        o.add("position",p);o.add("rotation",r);o.add("position_bits",pb);o.add("rotation_bits",rb);return o;
    }
    static double n(JsonObject p,int i){return p.getAsJsonArray("position").get(i).getAsDouble();}
    static float r(JsonObject p,int i){return p.getAsJsonArray("rotation").get(i).getAsFloat();}
    static void spawn(int id,List<JsonObject> actions){LivingEntity e=switch(actorKind){case 6->new ContactCat();case 7->new ContactDolphin();case 8->new ContactGiant();case 9->new ContactMooshroom();case 10->new ContactOcelot();case 11->new ContactPanda();case 12->new ContactSilverfish();case 13->new ContactVillager();case 14->new ContactHorse();case 4->new RemoteSheep();case 5->new RemoteCreeper();case 1->new RemotePig();case 2->new RemoteZombie();case 3->new RemoteActorPlayer(id);default->new RemoteCow();};int fresh=++lifetime;keys.put(e,fresh);e.setId(id);e.setPos(.1,65.2,boatMode?1.8:-.3);e.setDeltaMovement(.02,.0025,-.035);e.yRot=0;e.xRot=0;e.setPacketCoordinates(.1,65.2,boatMode?1.8:-.3);e.inChunk=true;e.xChunk=0;e.yChunk=4;e.zChunk=-1;byId.put(id,e);all.put(fresh,e);legacyMap.put(id,e);registered.getChunk(e.blockPosition()).addEntity(e);admitted.add(e);
        JsonObject a=new JsonObject();a.addProperty("op","spawn");a.addProperty("native_id",id);a.addProperty("key",key(e));a.addProperty("type_name",name(e));a.add("pose",pose(e));actions.add(a);
    }
    static JsonObject action(String op,int id){JsonObject a=new JsonObject();a.addProperty("op",op);a.addProperty("native_id",id);return a;}
    @SuppressWarnings("unchecked") static void sleep(LivingEntity e,boolean active,List<JsonObject> actions,String[] args)throws Exception {
        Field f=LivingEntity.class.getDeclaredField(args[18]);f.setAccessible(true);
        net.minecraft.network.syncher.EntityDataAccessor<Optional<net.minecraft.core.BlockPos>> accessor=(net.minecraft.network.syncher.EntityDataAccessor<Optional<net.minecraft.core.BlockPos>>)f.get(null);
        Optional<net.minecraft.core.BlockPos> value=active?Optional.of(new net.minecraft.core.BlockPos(0,65,0)):Optional.empty();
        net.minecraft.network.syncher.SynchedEntityData donor=new net.minecraft.network.syncher.SynchedEntityData(e);donor.define(accessor,value);listener.handleSetEntityData(new ClientboundSetEntityDataPacket(e.getId(),donor,true));
        JsonObject a=action("sleep",e.getId());a.add("position",active?new Gson().toJsonTree(new int[]{0,65,0}):JsonNull.INSTANCE);actions.add(a);
    }
    @SuppressWarnings("unchecked") static void baby(LivingEntity e,boolean active,List<JsonObject> actions,String[] args)throws Exception {
        Field f=AgableMob.class.getDeclaredField(args[19]);f.setAccessible(true);
        net.minecraft.network.syncher.EntityDataAccessor<Boolean> accessor=(net.minecraft.network.syncher.EntityDataAccessor<Boolean>)f.get(null);
        net.minecraft.network.syncher.SynchedEntityData donor=new net.minecraft.network.syncher.SynchedEntityData(e);donor.define(accessor,active);listener.handleSetEntityData(new ClientboundSetEntityDataPacket(e.getId(),donor,true));
        JsonObject a=action("baby",e.getId());a.addProperty("value",active);a.add("bounds",pose(e).get("bounds"));actions.add(a);
    }
    static void teleport(LivingEntity e,double distance,List<JsonObject> actions){
        double x=e.getX()+distance,y=e.getY(),z=e.getZ();float yaw=90,pitch=-45;
        JsonObject a=action("absolute",e.getId());a.add("position",new Gson().toJsonTree(new double[]{x,y,z}));a.add("rotation",new Gson().toJsonTree(new float[]{yaw,pitch}));a.addProperty("ground",true);actions.add(a);
        double ox=e.getX(),oy=e.getY(),oz=e.getZ();float or=e.yRot,op=e.xRot;boolean ground=e.isOnGround();
        e.setPos(x,y,z);e.yRot=yaw;e.xRot=pitch;e.setOnGround(true);ClientboundTeleportEntityPacket packet=new ClientboundTeleportEntityPacket(e);e.setPos(ox,oy,oz);e.yRot=or;e.xRot=op;e.setOnGround(ground);listener.handleTeleportEntity(packet);
    }
    static JsonObject evaluate(int seed,ServerLevel level,String[] args)throws Exception{
        controlledBoat=null;lifetime=0;keys=new IdentityHashMap<>();unticking=new HashSet<>();actorKind=seed<16?seed%4:seed<20?4:seed<24?5:6+(seed-24)/4;registered=level;byId=new LinkedHashMap<>();all=new LinkedHashMap<>();mutation=false;
        client=(ClientLevel)allocate(ProbeClientLevel.class);
        for(Field f:Level.class.getDeclaredFields())if(!Modifier.isStatic(f.getModifiers())){f.setAccessible(true);f.set(client,f.get(level));}
        admitted=new ArrayList<>();legacyMap=new Int2ObjectOpenHashMap<>();set(client,ClientLevel.class,args[1],legacyMap);set(client,ClientLevel.class,args[2],new ArrayList<>());set(client,ClientLevel.class,args[3],allocate(ProbeChunkCache.class));
        Minecraft minecraft=(Minecraft)allocate(Minecraft.class);
        set(minecraft,Minecraft.class,args[4],Thread.currentThread());
        set(client,ClientLevel.class,args[17],minecraft);
        listener=(ClientPacketListener)allocate(ClientPacketListener.class);
        set(client,ClientLevel.class,args[14],new ClientLevel.ClientLevelData(net.minecraft.world.Difficulty.NORMAL,false,true));
        set(listener,ClientPacketListener.class,args[5],minecraft);set(listener,ClientPacketListener.class,args[6],client);set(client,ClientLevel.class,args[8],new net.minecraft.world.scores.Scoreboard());set(client,ClientLevel.class,args[9],listener);

        JsonArray out=new JsonArray();int count=seed<24?new int[]{8,25,50,100}[seed%4]:8;int primary=seed*257+1;
        for(int stage=0;stage<12;stage++){
            List<JsonObject> actions=new ArrayList<>();inside=new ArrayList<>();seen=new ArrayList<>();
            if(stage==0 && seed%2==0){for(int i=0;i<count;i++)spawn(i==0?0:(i==1?primary:(seed==0&&i<5?new int[]{0,0,33,755,66}[i]:(i%2==0?-1:1)*(seed*257+i*37))),actions);}if(stage==0){ProbeLocalPlayer player=local(new ProbePlayer(level,2000000+seed),minecraft,args);exclusionLocal=player;legacyMap.put(player.getId(),player);registered.getChunk(player.blockPosition()).addEntity(player);admitted.add(player);JsonObject p=action("spawn_player",player.getId());p.addProperty("key",player.key);p.add("pose",pose(player));actions.add(p);}
            if(stage==0 && boatMode){controlledBoat=new ProbeBoat();legacyMap.put(controlledBoat.getId(),controlledBoat);registered.getChunk(controlledBoat.blockPosition()).addEntity(controlledBoat);admitted.add(controlledBoat);((ProbeLocalPlayer)minecraft.player).startRiding(controlledBoat);JsonObject a=action("spawn_boat",controlledBoat.getId());a.addProperty("key",controlledBoat.key);a.add("pose",pose(controlledBoat));actions.add(a);}
                        if(stage==0 && seed%2!=0){for(int i=0;i<count;i++)spawn(i==0?0:(i==1?primary:(seed==0&&i<5?new int[]{0,0,33,755,66}[i]:(i%2==0?-1:1)*(seed*257+i*37))),actions);}
            if(boatMode){boolean forward=stage<9;controlledBoat.setInput(false,false,forward,false);JsonObject a=action("boat_input",controlledBoat.getId());a.addProperty("forward",forward);actions.add(a);}
            LivingEntity e=byId.get(primary);
            if(actorKind==4 && (stage==0||stage==2))baby(e,stage==0,actions,args);
            if(stage==1||stage==10){for(int i=0;i<2;i++){
                JsonObject a=action("relative",primary);a.add("delta",new Gson().toJsonTree(new int[]{512,-128,-512}));a.add("rotation",new Gson().toJsonTree(new int[]{64,-32}));a.addProperty("ground",true);actions.add(a);
                listener.handleMoveEntity(new ClientboundMoveEntityPacket.PosRot(primary,(short)512,(short)-128,(short)-512,(byte)64,(byte)-32,true));}}
            if(stage==2){JsonObject a=action("relative",primary);a.add("delta",new Gson().toJsonTree(new int[]{0,0,1}));a.addProperty("ground",false);actions.add(a);listener.handleMoveEntity(new ClientboundMoveEntityPacket.Pos(primary,(short)0,(short)0,(short)1,false));
                a=action("rotation",primary);a.add("rotation",new Gson().toJsonTree(new int[]{-64,32}));a.addProperty("ground",true);actions.add(a);listener.handleMoveEntity(new ClientboundMoveEntityPacket.Rot(primary,(byte)-64,(byte)32,true));}
            if(stage==3){teleport(e,seed%2==0?.001:.03125,actions);JsonObject a=action("cancel",primary);actions.add(a);cancelModel(e);}
            if(stage==2||stage==3){net.minecraft.world.scores.Scoreboard board=client.getScoreboard();net.minecraft.world.scores.PlayerTeam team=board.getPlayerTeam("OwnerContact");if(team==null)team=board.addPlayerTeam("OwnerContact");team.setCollisionRule(stage==2?net.minecraft.world.scores.Team.CollisionRule.NEVER:net.minecraft.world.scores.Team.CollisionRule.ALWAYS);board.addPlayerToTeam(e.getScoreboardName(),team);JsonObject a=action("team",primary);a.addProperty("rule",stage==2?"never":"always");actions.add(a);}
            if(stage==4||stage==7)sleep(e,stage==4,actions,args);
            if(stage==5){JsonObject a=action("local",primary);a.addProperty("value",false);actions.add(a);listener.handleMoveEntity(new ClientboundMoveEntityPacket.Pos(primary,(short)1,(short)0,(short)0,false));a=action("relative",primary);a.add("delta",new Gson().toJsonTree(new int[]{1,0,0}));a.addProperty("ground",false);actions.add(a);}
            if(stage==6){net.minecraft.world.phys.Vec3 velocity=new net.minecraft.world.phys.Vec3(.125,-.25,.375);listener.handleSetEntityMotion(new ClientboundSetEntityMotionPacket(primary,velocity));JsonObject a=action("velocity",primary);a.add("velocity",new Gson().toJsonTree(new double[]{.125,-.25,.375}));actions.add(a);}
            if(stage==8){unticking.add(e.getId());actions.add(action("unticking",primary));}
            if(stage==10){teleport(e,seed%2==0?64.:65.,actions);teleport(e,0.,actions);}
            if(stage==11){cancelModel(e);actions.add(action("cancel",primary));}
            client.tickEntities();
            JsonObject step=new JsonObject();step.addProperty("stage",stage);step.add("actions",new Gson().toJsonTree(actions));step.add("during_tick",new Gson().toJsonTree(inside));step.add("ticks",new Gson().toJsonTree(seen));JsonArray states=new JsonArray();
            for(LivingEntity body:byId.values()){JsonObject state=new JsonObject();state.addProperty("key",key(body));state.addProperty("native_id",body.getId());state.add("pose",pose(body));state.addProperty("remaining_steps",remaining(body));states.add(state);}step.add("states",states);step.add("local_after",pose(minecraft.player));out.add(step);
        }
        for(Entity body:admitted)registered.getChunk(body.xChunk,body.zChunk).removeEntity(body);
        JsonObject row=new JsonObject();row.addProperty("seed",seed);row.addProperty("scene",boatMode?"boat":"walking");row.add("steps",out);return row;
    }
    static Player exclusionLocal;
    static JsonArray exclusions(String[] args) throws Exception {
        JsonArray rows=new JsonArray();
        Entity[] entities={new net.minecraft.world.entity.ambient.Bat(EntityType.BAT,client),new net.minecraft.world.entity.decoration.ArmorStand(EntityType.ARMOR_STAND,client),new net.minecraft.world.entity.item.ItemEntity(EntityType.ITEM,client),new net.minecraft.world.entity.projectile.Arrow(EntityType.ARROW,client)};
        String[] names={"minecraft:bat","minecraft:armor_stand","minecraft:item","minecraft:arrow"};
        Method doPush=LivingEntity.class.getDeclaredMethod(args[20],Entity.class);doPush.setAccessible(true);
        Method pushEntities=LivingEntity.class.getDeclaredMethod(args[21]);pushEntities.setAccessible(true);
        for(int i=0;i<entities.length;i++){
            Entity e=entities[i];e.setPos(.1,65.,-.3);
            JsonObject row=new JsonObject();row.addProperty("type",names[i]);row.addProperty("pushable",e.isPushable());
            exclusionLocal.setPos(.4,65.,-.3);exclusionLocal.setDeltaMovement(.125,-.25,.375);
            row.add("before",pose(exclusionLocal));
            if(e instanceof LivingEntity living){doPush.invoke(living,exclusionLocal);pushEntities.invoke(living);}
            row.add("after",pose(exclusionLocal));rows.add(row);
        }
        return rows;
    }
    public static void main(String[] args){try {
        originalPlayerAiStep=java.lang.invoke.MethodHandles.privateLookupIn(Player.class,java.lang.invoke.MethodHandles.lookup()).findSpecial(Player.class,args[10],java.lang.invoke.MethodType.methodType(void.class),Player.class);
        steps=LivingEntity.class.getDeclaredField(args[15]);steps.setAccessible(true);
        net.minecraft.server.Main.main(new String[]{"--nogui"});MinecraftServer server=null;
        for(int i=0;i<120&&(server==null||server.getTickCount()<20);i++){Thread.sleep(500);if(server==null)server=findServer();}
        if(server==null)throw new IllegalStateException("not ready");MinecraftServer s=server;
        JsonArray result=new JsonArray();s.executeBlocking(()->{for(int x=-16;x<=32;x++)for(int z=-16;z<=16;z++)s.overworld().setBlock(new net.minecraft.core.BlockPos(x,64,z),net.minecraft.world.level.block.Blocks.STONE.defaultBlockState(),0);try{set(s.overworld(),Level.class,args[7],true);try{for(int scene=0;scene<2;scene++){boatMode=scene==1;for(int seed=0;seed<60;seed++)result.add(evaluate(seed,s.overworld(),args));}Files.writeString(Path.of(args[0]+".exclusions.json"),new Gson().toJson(exclusions(args))+"\n");}finally{set(s.overworld(),Level.class,args[7],false);}}catch(Exception e){throw new RuntimeException(e);}});
        Files.writeString(Path.of(args[0]),new Gson().toJson(result)+"\n");s.halt(false);System.exit(0);
    }catch(Throwable e){e.printStackTrace();System.exit(1);}}
}
