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
import net.minecraft.world.level.entity.TransientEntitySectionManager;
import net.minecraft.world.level.entity.LevelCallback;
import net.minecraft.world.level.Level;
import net.minecraft.client.Minecraft;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.network.PacketProcessor;
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
    static boolean mutation;
    static List<JsonObject> inside;
    static ServerLevel registered;
    static Int2ObjectOpenHashMap<Entity> legacyMap;
    static EntityTickList modernList;
    static TickRateManager rates;
    static Field data,steps;
    static TransientEntitySectionManager<Entity> storage;
    static class ProbeChunkCache extends ClientChunkCache {
        ProbeChunkCache(){super(null,4);}
        @Override public LevelChunk getChunk(int x,int z,net.minecraft.world.level.chunk.status.ChunkStatus status,boolean must){return (LevelChunk)registered.getChunk(x,z,status,must);}
        @Override public net.minecraft.world.level.lighting.LevelLightEngine getLightEngine(){return registered.getChunkSource().getLightEngine();}
    }
    static class Callbacks implements LevelCallback<Entity>{
        public void onCreated(Entity e){} public void onDestroyed(Entity e){}
        public void onTrackingStart(Entity e){} public void onTrackingEnd(Entity e){}
        public void onTickingStart(Entity e){modernList.add(e);} public void onTickingEnd(Entity e){modernList.remove(e);}
        public void onSectionChange(Entity e){}
    }
    static Map<UUID,LivingEntity> teamActors;
    static class ProbeClientLevel extends ClientLevel {
        @Override public Entity getEntity(UUID id){return teamActors!=null?teamActors.get(id):super.getEntity(id);}
        @Override public Player getPlayerByUUID(UUID id){if(teamActors!=null){LivingEntity e=teamActors.get(id);return e instanceof Player?(Player)e:null;}return super.getPlayerByUUID(id);}
        @Override public void addParticle(net.minecraft.core.particles.ParticleOptions p,double x,double y,double z,double vx,double vy,double vz){}
        ProbeClientLevel(){super(null,null,null,null,4,4,null,false,0,63);}
        @Override public Entity getEntity(int id){return controlledBoat!=null && controlledBoat.getId()==id?controlledBoat:byId.get(id);}
        @Override public void sendPacketToServer(net.minecraft.network.protocol.Packet<?> packet){}
        @Override public void playLocalSound(double x,double y,double z,net.minecraft.sounds.SoundEvent sound,net.minecraft.sounds.SoundSource category,float volume,float pitch,boolean distance){}
        @Override public void playLocalSound(Entity entity,net.minecraft.sounds.SoundEvent sound,net.minecraft.sounds.SoundSource category,float volume,float pitch){}
    }
    static class ProbePlayer extends Player {
        final int key;
        ProbePlayer(ServerLevel level,int id){super(level,new GameProfile(new UUID(0,id),"OraclePlayer"));key=++lifetime;setId(id);setPos(.4,65.0,-.3);setDeltaMovement(.02,.0025,-.035);setYRot(0);setXRot(0);setLevel(client);}
        @Override public net.minecraft.world.level.GameType gameMode(){return net.minecraft.world.level.GameType.SURVIVAL;}
        @Override public boolean isLocalPlayer(){return true;}
        @Override public void tick(){JsonObject e=new JsonObject();e.addProperty("key",key);e.addProperty("native_id",getId());e.addProperty("dispatch","player_tick");e.add("before",pose(this));e.add("after",pose(this));seen.add(e);}
    }
    static java.lang.invoke.MethodHandle originalPlayerAiStep;
    static class ProbeLocalPlayer extends LocalPlayer {
        int key;
        ProbeLocalPlayer(){super(null,null,null,null,null,null,false);}
        @Override public net.minecraft.world.level.GameType gameMode(){return net.minecraft.world.level.GameType.SURVIVAL;}
        @Override public void rideTick(){}
        @Override public void tick(){JsonObject e=new JsonObject();e.addProperty("key",key);e.addProperty("native_id",getId());e.addProperty("dispatch","player_tick");e.add("before",pose(this));try{originalPlayerAiStep.invokeExact((Player)this);}catch(Throwable error){throw new RuntimeException(error);}e.add("after",pose(this));seen.add(e);}
    }
    static ProbeLocalPlayer local(ProbePlayer seed,Minecraft minecraft,String[] args)throws Exception {
        ProbeLocalPlayer body=(ProbeLocalPlayer)allocate(ProbeLocalPlayer.class);
        for(Class<?> k=Player.class;k!=Object.class;k=k.getSuperclass())for(Field f:k.getDeclaredFields())if(!Modifier.isStatic(f.getModifiers())){f.setAccessible(true);f.set(body,f.get(seed));}
        body.key=seed.key;set(body.getEntityData(),net.minecraft.network.syncher.SynchedEntityData.class,args[23],body);body.input=new net.minecraft.client.player.ClientInput();set(body,net.minecraft.client.player.AbstractClientPlayer.class,args[22],new net.minecraft.client.entity.ClientAvatarState());set(body,LocalPlayer.class,args[16],minecraft);set(body,LocalPlayer.class,args[17],listener);set(minecraft,Minecraft.class,args[15],body);return body;
    }
    static class ProbeBoat extends net.minecraft.world.entity.vehicle.boat.Boat {
        int key;
        ProbeBoat(){super(EntityType.OAK_BOAT,client,()->net.minecraft.world.item.Items.OAK_BOAT);key=++lifetime;setId(3000000);setPos(.4,65.,-.3);setYRot(0);setXRot(0);setDeltaMovement(.02,0.,.08);}
        @Override public void tick(){JsonObject e=new JsonObject();e.addProperty("key",key);e.addProperty("native_id",getId());e.addProperty("dispatch","boat_tick");e.add("before",pose(this));net.minecraft.world.phys.AABB box=getBoundingBox();box=new net.minecraft.world.phys.AABB(box.minX-1.0000001,box.minY-5.0000001,box.minZ-1.0000001,box.maxX+1.0000001,box.maxY+5.0000001,box.maxZ+1.0000001);JsonArray candidates=new JsonArray();for(Entity other:client.getEntities(this,box,EntitySelector.NO_SPECTATORS)){if(other instanceof LivingEntity living && keys.containsKey(living) && other.isPushable())candidates.add(key(living));}e.add("geometry_query",candidates);super.tick();e.add("after",pose(this));seen.add(e);}
    }
    static class ContactAxolotl extends net.minecraft.world.entity.animal.axolotl.Axolotl {
        ContactAxolotl(){super(EntityType.AXOLOTL,client);}
        @Override public void tick(){dispatch(this,()->super.aiStep());}
    }
    static class ContactBogged extends net.minecraft.world.entity.monster.skeleton.Bogged {
        ContactBogged(){super(EntityType.BOGGED,client);}
        @Override public void tick(){dispatch(this,()->super.aiStep());}
    }
    static class ContactCat extends net.minecraft.world.entity.animal.feline.Cat {
        ContactCat(){super(EntityType.CAT,client);}
        @Override public void tick(){dispatch(this,()->super.aiStep());}
    }
    static class ContactCopperGolem extends net.minecraft.world.entity.animal.golem.CopperGolem {
        ContactCopperGolem(){super(EntityType.COPPER_GOLEM,client);}
        @Override public void tick(){dispatch(this,()->super.aiStep());}
    }
    static class ContactGiant extends net.minecraft.world.entity.monster.Giant {
        ContactGiant(){super(EntityType.GIANT,client);}
        @Override public void tick(){dispatch(this,()->super.aiStep());}
    }
    static class ContactOcelot extends net.minecraft.world.entity.animal.feline.Ocelot {
        ContactOcelot(){super(EntityType.OCELOT,client);}
        @Override public void tick(){dispatch(this,()->super.aiStep());}
    }
    static class ContactParched extends net.minecraft.world.entity.monster.skeleton.Parched {
        ContactParched(){super(EntityType.PARCHED,client);}
        @Override public void tick(){dispatch(this,()->super.aiStep());}
    }
    static class ContactPiglinBrute extends net.minecraft.world.entity.monster.piglin.PiglinBrute {
        ContactPiglinBrute(){super(EntityType.PIGLIN_BRUTE,client);}
        @Override public void tick(){dispatch(this,()->super.aiStep());}
    }
    static class ContactSilverfish extends net.minecraft.world.entity.monster.Silverfish {
        ContactSilverfish(){super(EntityType.SILVERFISH,client);}
        @Override public void tick(){dispatch(this,()->super.aiStep());}
    }
    static class ContactSkeleton extends net.minecraft.world.entity.monster.skeleton.Skeleton {
        ContactSkeleton(){super(EntityType.SKELETON,client);}
        @Override public void tick(){dispatch(this,()->super.aiStep());}
    }
    static class ContactStray extends net.minecraft.world.entity.monster.skeleton.Stray {
        ContactStray(){super(EntityType.STRAY,client);}
        @Override public void tick(){dispatch(this,()->super.aiStep());}
    }
    static class ContactVillager extends net.minecraft.world.entity.npc.villager.Villager {
        ContactVillager(){super(EntityType.VILLAGER,client);}
        @Override public void tick(){dispatch(this,()->super.aiStep());}
    }
    static class ContactWitherSkeleton extends net.minecraft.world.entity.monster.skeleton.WitherSkeleton {
        ContactWitherSkeleton(){super(EntityType.WITHER_SKELETON,client);}
        @Override public void tick(){dispatch(this,()->super.aiStep());}
    }
    static class ContactMooshroom extends net.minecraft.world.entity.animal.cow.MushroomCow {
        ContactMooshroom(){super(EntityType.MOOSHROOM,client);}
        @Override public void tick(){dispatch(this,()->super.aiStep());}
    }
    // Execute the unchanged Wolf.tick wrapper; inherited AI/travel/baseTick
    // remain isolated exactly as the private numerical owner contract requires.
    static class StateWolf extends net.minecraft.world.entity.animal.wolf.Wolf {
        boolean wetInput;
        StateWolf(){super(EntityType.WOLF,client);}
        @Override public void aiStep(){}
        @Override public void baseTick(){}
        @Override public boolean isInWaterOrRain(){return wetInput;}
    }
    static class ContactWolf extends net.minecraft.world.entity.animal.wolf.Wolf {
        ContactWolf(){super(EntityType.WOLF,client);}
        @Override public void tick(){dispatch(this,()->super.aiStep());}
    }
    static class ContactChicken extends net.minecraft.world.entity.animal.chicken.Chicken {
        ContactChicken(){super(EntityType.CHICKEN,client);}
        @Override public void tick(){dispatch(this,()->super.aiStep());}
    }
    static class ContactRabbit extends net.minecraft.world.entity.animal.rabbit.Rabbit {
        ContactRabbit(){super(EntityType.RABBIT,client);}
        @Override public void tick(){dispatch(this,()->super.aiStep());}
    }
    static class ContactWanderingTrader extends net.minecraft.world.entity.npc.wanderingtrader.WanderingTrader {
        ContactWanderingTrader(){super(EntityType.WANDERING_TRADER,client);}
        @Override public void tick(){dispatch(this,()->super.aiStep());}
    }
    static class ContactHorse extends net.minecraft.world.entity.animal.equine.Horse {
        ContactHorse(){super(EntityType.HORSE,client);}
        @Override public void tick(){dispatch(this,()->super.aiStep());}
    }
    static class RemoteSheep extends net.minecraft.world.entity.animal.sheep.Sheep {
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
    static class RemotePig extends net.minecraft.world.entity.animal.pig.Pig {
        RemotePig(){super(EntityType.PIG,client);}
        @Override public void tick(){dispatch(this,()->super.aiStep());}
    }
    static class RemoteZombie extends net.minecraft.world.entity.monster.zombie.Zombie {
        RemoteZombie(){super(EntityType.ZOMBIE,client);}
        @Override public void tick(){dispatch(this,()->super.aiStep());}
    }
    static class RemoteActorPlayer extends net.minecraft.client.player.RemotePlayer {
        RemoteActorPlayer(int id){super(client,new GameProfile(new UUID(1,id),"Remote"+id));}
        @Override public net.minecraft.world.level.GameType gameMode(){return net.minecraft.world.level.GameType.SURVIVAL;}
        @Override public void tick(){dispatch(this,()->super.aiStep());}
    }
    static int key(LivingEntity e){return keys.get(e);}
    static int remaining(LivingEntity e){try{return steps.getInt(data.get(e.getInterpolation()));}catch(Exception error){throw new RuntimeException(error);}}
    static void cancelModel(LivingEntity e){e.getInterpolation().cancel();}
    static String name(LivingEntity e){return EntityType.getKey(e.getType()).toString();}
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
        JsonObject p=pose(e.getX(),e.getY(),e.getZ(),e.getYRot(),e.getXRot());JsonArray velocity=new JsonArray(),bits=new JsonArray();for(double n:new double[]{e.getDeltaMovement().x,e.getDeltaMovement().y,e.getDeltaMovement().z}){velocity.add(n);bits.add(Long.toUnsignedString(Double.doubleToRawLongBits(n),16));}p.add("velocity",velocity);p.add("velocity_bits",bits);net.minecraft.world.phys.AABB b=e.getBoundingBox();p.add("bounds",new Gson().toJsonTree(new double[]{b.minX,b.minY,b.minZ,b.maxX,b.maxY,b.maxZ}));JsonArray boundsBits=new JsonArray();for(double n:new double[]{b.minX,b.minY,b.minZ,b.maxX,b.maxY,b.maxZ})boundsBits.add(Long.toUnsignedString(Double.doubleToRawLongBits(n),16));p.add("bounds_bits",boundsBits);if(e instanceof ContactChicken c){JsonArray flapBits=new JsonArray();for(float n:new float[]{c.flap,c.flapSpeed,c.oFlap,c.oFlapSpeed,c.flapping})flapBits.add(Integer.toUnsignedLong(Float.floatToRawIntBits(n)));p.add("chicken_bits",flapBits);}return p;
    }
    static JsonObject pose(double x,double y,double z,float yaw,float pitch) {
        JsonObject o=new JsonObject();JsonArray p=new JsonArray(),r=new JsonArray(),pb=new JsonArray(),rb=new JsonArray();
        for(double n:new double[]{x,y,z}){p.add(n);pb.add(Long.toUnsignedString(Double.doubleToRawLongBits(n),16));}
        for(float n:new float[]{yaw,pitch}){r.add((double)n);rb.add(Integer.toUnsignedLong(Float.floatToRawIntBits(n)));}
        o.add("position",p);o.add("rotation",r);o.add("position_bits",pb);o.add("rotation_bits",rb);return o;
    }
    static double n(JsonObject p,int i){return p.getAsJsonArray("position").get(i).getAsDouble();}
    static float r(JsonObject p,int i){return p.getAsJsonArray("rotation").get(i).getAsFloat();}
    static void spawn(int id,List<JsonObject> actions){LivingEntity e=switch(actorKind){case 24->new ContactWolf();case 23->new ContactChicken();case 20->new ContactMooshroom();case 21->new ContactRabbit();case 22->new ContactWanderingTrader();case 6->new ContactAxolotl();case 7->new ContactBogged();case 8->new ContactCat();case 9->new ContactCopperGolem();case 10->new ContactGiant();case 11->new ContactOcelot();case 12->new ContactParched();case 13->new ContactPiglinBrute();case 14->new ContactSilverfish();case 15->new ContactSkeleton();case 16->new ContactStray();case 17->new ContactVillager();case 18->new ContactWitherSkeleton();case 19->new ContactHorse();case 4->new RemoteSheep();case 5->new RemoteCreeper();case 1->new RemotePig();case 2->new RemoteZombie();case 3->new RemoteActorPlayer(id);default->new RemoteCow();};int fresh=++lifetime;keys.put(e,fresh);e.setId(id);e.setPos(.1,65.2,boatMode?1.8:-.3);e.setDeltaMovement(.02,.0025,-.035);e.setYRot(0);e.setXRot(0);e.getPositionCodec().setBase(e.position());byId.put(id,e);all.put(fresh,e);storage.addEntity(e);
        JsonObject a=new JsonObject();a.addProperty("op","spawn");a.addProperty("native_id",id);a.addProperty("key",key(e));a.addProperty("type_name",name(e));a.add("pose",pose(e));actions.add(a);
    }
    static JsonObject action(String op,int id){JsonObject a=new JsonObject();a.addProperty("op",op);a.addProperty("native_id",id);return a;}
    @SuppressWarnings("unchecked") static void sleep(LivingEntity e,boolean active,List<JsonObject> actions,String[] args)throws Exception {
        Field f=LivingEntity.class.getDeclaredField(args[25]);f.setAccessible(true);
        net.minecraft.network.syncher.EntityDataAccessor<Optional<net.minecraft.core.BlockPos>> accessor=(net.minecraft.network.syncher.EntityDataAccessor<Optional<net.minecraft.core.BlockPos>>)f.get(null);
        Optional<net.minecraft.core.BlockPos> value=active?Optional.of(new net.minecraft.core.BlockPos(0,65,0)):Optional.empty();
        listener.handleSetEntityData(new ClientboundSetEntityDataPacket(e.getId(),List.of(net.minecraft.network.syncher.SynchedEntityData.DataValue.create(accessor,value))));
        JsonObject a=action("sleep",e.getId());a.add("position",active?new Gson().toJsonTree(new int[]{0,65,0}):JsonNull.INSTANCE);actions.add(a);
    }
    @SuppressWarnings("unchecked") static void baby(LivingEntity e,boolean active,List<JsonObject> actions,String[] args)throws Exception {
        Field f=AgeableMob.class.getDeclaredField(args[26]);f.setAccessible(true);
        net.minecraft.network.syncher.EntityDataAccessor<Boolean> accessor=(net.minecraft.network.syncher.EntityDataAccessor<Boolean>)f.get(null);
        listener.handleSetEntityData(new ClientboundSetEntityDataPacket(e.getId(),List.of(net.minecraft.network.syncher.SynchedEntityData.DataValue.create(accessor,active))));
        JsonObject a=action("baby",e.getId());a.addProperty("value",active);a.addProperty("type_name",name(e));a.add("bounds",pose(e).get("bounds"));actions.add(a);
    }
    // Exercise species metadata through the unchanged original packet handler.
    @SuppressWarnings({"unchecked","rawtypes"}) static void variant(LivingEntity e,boolean alternate,List<JsonObject> actions)throws Exception {
        Object value;
        if(e instanceof ContactChicken)value=registered.registryAccess().lookupOrThrow(net.minecraft.core.registries.Registries.CHICKEN_VARIANT).getOrThrow(alternate?net.minecraft.world.entity.animal.chicken.ChickenVariants.COLD:net.minecraft.world.entity.animal.chicken.ChickenVariants.WARM);
        else if(e instanceof ContactRabbit)value=alternate?99:0;
        else if(e instanceof ContactOcelot)value=alternate;
        else if(e instanceof ContactMooshroom)value=alternate?1:0;
        else if(e instanceof ContactVillager villager)value=villager.getVillagerData().withLevel(alternate?5:1);
        else return;
        Class<?> original=e.getClass().getSuperclass();
        net.minecraft.network.syncher.EntityDataAccessor accessor=null;
        for(Field f:original.getDeclaredFields())if(Modifier.isStatic(f.getModifiers()) && f.getType()==net.minecraft.network.syncher.EntityDataAccessor.class){
            if(accessor!=null)throw new IllegalStateException("ambiguous variant accessor");
            f.setAccessible(true);accessor=(net.minecraft.network.syncher.EntityDataAccessor)f.get(null);
        }
        if(accessor==null)throw new IllegalStateException("missing variant accessor");
        listener.handleSetEntityData(new ClientboundSetEntityDataPacket(e.getId(),List.of(net.minecraft.network.syncher.SynchedEntityData.DataValue.create(accessor,value))));
        JsonObject a=action("variant",e.getId());a.addProperty("type_name",name(e));a.addProperty("index",accessor.id());
        a.addProperty("serializer",net.minecraft.network.syncher.EntityDataSerializers.getSerializedId(accessor.serializer()));a.addProperty("alternate",alternate);a.add("pose",pose(e));actions.add(a);
    }
    @SuppressWarnings("unchecked") static void poseMetadata(LivingEntity e,boolean sleeping,List<JsonObject> actions)throws Exception {
        net.minecraft.network.syncher.EntityDataAccessor<net.minecraft.world.entity.Pose> accessor=null;
        for(Field f:Entity.class.getDeclaredFields())if(Modifier.isStatic(f.getModifiers()) && f.getType()==net.minecraft.network.syncher.EntityDataAccessor.class){
            f.setAccessible(true);net.minecraft.network.syncher.EntityDataAccessor<?> candidate=(net.minecraft.network.syncher.EntityDataAccessor<?>)f.get(null);
            if(e.getEntityData().get(candidate) instanceof net.minecraft.world.entity.Pose)accessor=(net.minecraft.network.syncher.EntityDataAccessor<net.minecraft.world.entity.Pose>)candidate;
        }
        if(accessor==null)throw new IllegalStateException("missing pose accessor");
        net.minecraft.world.entity.Pose value=sleeping?net.minecraft.world.entity.Pose.SLEEPING:net.minecraft.world.entity.Pose.STANDING;
        listener.handleSetEntityData(new ClientboundSetEntityDataPacket(e.getId(),List.of(net.minecraft.network.syncher.SynchedEntityData.DataValue.create(accessor,value))));
        JsonObject a=action("pose_metadata",e.getId());a.addProperty("type_name",name(e));a.addProperty("native_pose",value.ordinal());a.add("pose",pose(e));actions.add(a);
    }
    static void teleport(LivingEntity e,double distance,List<JsonObject> actions){
        double x=e.getX()+distance,y=e.getY(),z=e.getZ();float yaw=90,pitch=-45;
        JsonObject a=action("absolute",e.getId());a.add("position",new Gson().toJsonTree(new double[]{x,y,z}));a.add("rotation",new Gson().toJsonTree(new float[]{yaw,pitch}));a.addProperty("ground",true);actions.add(a);
        a.add("velocity",new Gson().toJsonTree(new double[]{.125,-.25,.375}));
        listener.handleEntityPositionSync(new ClientboundEntityPositionSyncPacket(e.getId(),new PositionMoveRotation(new net.minecraft.world.phys.Vec3(x,y,z),new net.minecraft.world.phys.Vec3(.125,-.25,.375),yaw,pitch),true));
    }
    static void wolfOwner(net.minecraft.world.entity.animal.wolf.Wolf w,UUID owner,boolean tame){w.setOwnerReference(owner==null?null:EntityReference.of(owner));w.setTame(tame,false);}
    static JsonArray wolfTeams(){
        JsonArray rows=new JsonArray();
        for(int scenario=0;scenario<12;scenario++){
            teamActors=new LinkedHashMap<>();
            ContactWolf a=new ContactWolf(),b=new ContactWolf();RemoteActorPlayer p=new RemoteActorPlayer(501);
            a.setUUID(new UUID(0,10));b.setUUID(new UUID(0,11));
            teamActors.put(a.getUUID(),a);teamActors.put(b.getUUID(),b);teamActors.put(p.getUUID(),p);
            boolean tame=scenario!=0;UUID owner=scenario==1?null:scenario==2?new UUID(0,999):scenario<6?p.getUUID():b.getUUID();
            UUID parentOwner=scenario==8?a.getUUID():scenario==9?b.getUUID():scenario==10?null:p.getUUID();
            wolfOwner(a,owner,tame);wolfOwner(b,parentOwner,scenario!=7);
            net.minecraft.world.scores.Scoreboard board=client.getScoreboard();
            String suffix="Wolf"+scenario;
            var at=board.addPlayerTeam("A"+suffix);var bt=board.addPlayerTeam("B"+suffix);var pt=board.addPlayerTeam("P"+suffix);
            boolean own=scenario!=4 && scenario!=6 && scenario!=7 && scenario!=8 && scenario!=9 && scenario!=10 && scenario!=11;
            if(own)board.addPlayerToTeam(a.getUUID().toString(),at);
            board.addPlayerToTeam(b.getUUID().toString(),bt);
            if(scenario!=5)board.addPlayerToTeam("Remote501",pt);
            JsonObject r=new JsonObject();r.addProperty("scenario",scenario);r.addProperty("tame",tame);r.addProperty("own_team",own);r.addProperty("parent_tame",scenario!=7);
            r.addProperty("owner",owner==null?null:owner.toString());r.addProperty("parent_owner",parentOwner==null?null:parentOwner.toString());
            r.addProperty("result",a.getTeam()==null?null:a.getTeam().getName());rows.add(r);
            if(own)board.removePlayerFromTeam(a.getUUID().toString(),at);board.removePlayerFromTeam(b.getUUID().toString(),bt);
            if(scenario!=5)board.removePlayerFromTeam("Remote501",pt);
        }
        teamActors=null;return rows;
    }
    static JsonArray wolfStates(String[] args)throws Exception {
        StateWolf w=new StateWolf();w.setPos(.1,65.2,-.3);w.setDeltaMovement(.02,-.3,-.035);
        JsonArray rows=new JsonArray();
        for(int tick=0;tick<80;tick++){
            w.wetInput=tick<2 || tick==12 || tick==22;
            w.setIsInterested(tick%7<4);w.setInSittingPose(tick%3==0);
            int event=tick==2||tick==10||tick==23?8:tick==15?56:0;
            if(event!=0)w.handleEntityEvent((byte)event);
            JsonObject r=new JsonObject();r.addProperty("wet_input",w.wetInput);r.addProperty("interested",tick%7<4);r.addProperty("event",event);
            r.add("before",pose(w));w.tick();r.add("after",pose(w));
            JsonArray bits=new JsonArray();for(int i=0;i<4;i++){Field f=net.minecraft.world.entity.animal.wolf.Wolf.class.getDeclaredField(args[args.length-6+i]);f.setAccessible(true);bits.add(Integer.toUnsignedLong(Float.floatToRawIntBits(f.getFloat(w))));}
            r.add("bits",bits);
            for(int i=4;i<6;i++){Field f=net.minecraft.world.entity.animal.wolf.Wolf.class.getDeclaredField(args[args.length-6+i]);f.setAccessible(true);r.addProperty(i==4?"wet":"shaking",f.getBoolean(w));}
            rows.add(r);
        }
        return rows;
    }
    static JsonObject evaluate(int seed,ServerLevel level,String[] args)throws Exception{
        controlledBoat=null;lifetime=0;keys=new IdentityHashMap<>();unticking=new HashSet<>();actorKind=seed<16?seed%4:seed<20?4:seed<24?5:6+(seed-24)/4;registered=level;byId=new LinkedHashMap<>();all=new LinkedHashMap<>();mutation=false;
        client=(ClientLevel)allocate(ProbeClientLevel.class);
        for(Field f:Level.class.getDeclaredFields())if(!Modifier.isStatic(f.getModifiers())){f.setAccessible(true);f.set(client,f.get(level));}
        modernList=new EntityTickList();rates=new TickRateManager();set(client,ClientLevel.class,args[1],modernList);set(client,ClientLevel.class,args[2],rates);
        set(client,ClientLevel.class,args[24],registered.environmentAttributes());
        ClientLevel.ClientLevelData worldData=new ClientLevel.ClientLevelData(net.minecraft.world.Difficulty.NORMAL,false,true);worldData.setGameTime(registered.getGameTime());worldData.setDayTime(registered.getDayTime());set(client,ClientLevel.class,args[29],worldData);
        Minecraft minecraft=(Minecraft)allocate(Minecraft.class);
        set(minecraft,Minecraft.class,args[3],new PacketProcessor(Thread.currentThread()));
        listener=(ClientPacketListener)allocate(ClientPacketListener.class);
        set(listener,ClientCommonPacketListenerImpl.class,args[4],minecraft);set(listener,ClientPacketListener.class,args[5],client);set(client,ClientLevel.class,args[12],listener);set(client,ClientLevel.class,args[14],minecraft);set(listener,ClientPacketListener.class,args[13],new net.minecraft.world.scores.Scoreboard());

        set(client,ClientLevel.class,args[10],allocate(ProbeChunkCache.class));storage=new TransientEntitySectionManager<>(Entity.class,new Callbacks());set(client,ClientLevel.class,args[11],storage);
        for(int x=-4;x<=4;x++)for(int z=-4;z<=4;z++)storage.startTicking(new net.minecraft.world.level.ChunkPos(x,z));
        set(client,ClientLevel.class,args[18],new ArrayList<>());set(client,ClientLevel.class,args[19],new ArrayList<>());set(client,ClientLevel.class,args[20],registered.getWorldBorder());
        JsonArray out=new JsonArray();int count=seed<24?new int[]{8,25,50,100}[seed%4]:8;int primary=seed*257+1;
        for(int stage=0;stage<12;stage++){
            List<JsonObject> actions=new ArrayList<>();inside=new ArrayList<>();seen=new ArrayList<>();
            if(stage==0 && seed%2==0){for(int i=0;i<count;i++)spawn(i==0?0:(i==1?primary:(i%2==0?-1:1)*(seed*257+i*37)),actions);}if(stage==0){ProbeLocalPlayer player=local(new ProbePlayer(level,2000000+seed),minecraft,args);exclusionLocal=player;modernList.add(player);storage.addEntity(player);JsonObject p=action("spawn_player",player.getId());p.addProperty("key",player.key);p.add("pose",pose(player));actions.add(p);}
            if(stage==0 && boatMode){controlledBoat=new ProbeBoat();modernList.add(controlledBoat);storage.addEntity(controlledBoat);((ProbeLocalPlayer)minecraft.player).startRiding(controlledBoat);JsonObject a=action("spawn_boat",controlledBoat.getId());a.addProperty("key",controlledBoat.key);a.add("pose",pose(controlledBoat));actions.add(a);}
                        if(stage==0 && seed%2!=0){for(int i=0;i<count;i++)spawn(i==0?0:(i==1?primary:(i%2==0?-1:1)*(seed*257+i*37)),actions);}
            if(boatMode){boolean forward=stage<9;controlledBoat.setInput(false,false,forward,false);JsonObject a=action("boat_input",controlledBoat.getId());a.addProperty("forward",forward);actions.add(a);}
            LivingEntity e=byId.get(primary);
            if(e instanceof ContactWolf w){
                w.setInSittingPose(stage%3==0);w.setIsInterested(stage%2==0);
                JsonObject a=action("wolf_flags",e.getId());a.addProperty("sitting",stage%3==0);a.addProperty("interested",stage%2==0);a.add("pose",pose(e));actions.add(a);
            }
            if(stage==1||stage==3)variant(e,stage==1,actions);
            if((actorKind==4 || e instanceof ContactWolf || e instanceof ContactChicken || e instanceof ContactRabbit || e instanceof ContactOcelot || e instanceof ContactVillager || e instanceof ContactWanderingTrader || e instanceof ContactMooshroom) && (stage==0||stage==2))baby(e,stage==0,actions,args);
            if(stage==1||stage==10){for(int i=0;i<2;i++){
                JsonObject a=action("relative",primary);a.add("delta",new Gson().toJsonTree(new int[]{512,-128,-512}));a.add("rotation",new Gson().toJsonTree(new int[]{64,-32}));a.addProperty("ground",true);actions.add(a);
                listener.handleMoveEntity(new ClientboundMoveEntityPacket.PosRot(primary,(short)512,(short)-128,(short)-512,(byte)64,(byte)-32,true));}}
            if(stage==2){JsonObject a=action("relative",primary);a.add("delta",new Gson().toJsonTree(new int[]{0,0,1}));a.addProperty("ground",false);actions.add(a);listener.handleMoveEntity(new ClientboundMoveEntityPacket.Pos(primary,(short)0,(short)0,(short)1,false));
                a=action("rotation",primary);a.add("rotation",new Gson().toJsonTree(new int[]{-64,32}));a.addProperty("ground",true);actions.add(a);listener.handleMoveEntity(new ClientboundMoveEntityPacket.Rot(primary,(byte)-64,(byte)32,true));}
            if(stage==3){teleport(e,seed%2==0?.001:.03125,actions);JsonObject a=action("cancel",primary);actions.add(a);cancelModel(e);}
            if(stage==2||stage==3){net.minecraft.world.scores.Scoreboard board=client.getScoreboard();net.minecraft.world.scores.PlayerTeam team=board.getPlayerTeam("OwnerContact");if(team==null)team=board.addPlayerTeam("OwnerContact");team.setCollisionRule(stage==2?net.minecraft.world.scores.Team.CollisionRule.NEVER:net.minecraft.world.scores.Team.CollisionRule.ALWAYS);board.addPlayerToTeam(e.getScoreboardName(),team);JsonObject a=action("team",primary);a.addProperty("rule",stage==2?"never":"always");actions.add(a);}
            if((e instanceof ContactWolf || e instanceof ContactChicken || e instanceof ContactRabbit || e instanceof ContactOcelot || e instanceof ContactVillager || e instanceof ContactWanderingTrader || e instanceof ContactMooshroom || e instanceof ContactSilverfish || e instanceof ContactGiant) && stage==9){poseMetadata(e,true,actions);poseMetadata(e,false,actions);}
            if((e instanceof ContactWolf || e instanceof ContactChicken || e instanceof ContactRabbit || e instanceof ContactOcelot || e instanceof ContactVillager || e instanceof ContactWanderingTrader || e instanceof ContactMooshroom || e instanceof ContactSilverfish || e instanceof ContactGiant) && (stage==4||stage==7))poseMetadata(e,stage==4,actions);
            if(stage==4||stage==7)sleep(e,stage==4,actions,args);
            if(stage==5){JsonObject a=action("local",primary);a.addProperty("value",false);actions.add(a);listener.handleMoveEntity(new ClientboundMoveEntityPacket.Pos(primary,(short)1,(short)0,(short)0,false));a=action("relative",primary);a.add("delta",new Gson().toJsonTree(new int[]{1,0,0}));a.addProperty("ground",false);actions.add(a);}
            if(stage==6){net.minecraft.world.phys.Vec3 velocity=new net.minecraft.world.phys.Vec3(.125,-.25,.375);listener.handleSetEntityMotion(new ClientboundSetEntityMotionPacket(primary,velocity));JsonObject a=action("velocity",primary);a.add("velocity",new Gson().toJsonTree(new double[]{.125,-.25,.375}));actions.add(a);}
            if(e instanceof ContactChicken && (stage==0||stage==3||stage==7||stage==9)){
                double y=stage==0?-.25:stage==3?.25:stage==7?-.003:-.003125;
                net.minecraft.world.phys.Vec3 v=new net.minecraft.world.phys.Vec3(.125,y,.375);
                ClientboundSetEntityMotionPacket packet=new ClientboundSetEntityMotionPacket(primary,v);listener.handleSetEntityMotion(packet);
                JsonObject a=action("velocity",primary);a.add("velocity",new Gson().toJsonTree(new double[]{.125,((int)(y*8000))/8000.,.375}));actions.add(a);
            }
            if(stage==8 && !boatMode){rates.setFrozen(true);rates.tick();actions.add(action("freeze",primary));}
            if(stage==10){teleport(e,seed%2==0?64.:65.,actions);teleport(e,0.,actions);}
            if(stage==11){cancelModel(e);actions.add(action("cancel",primary));}
            client.tickEntities();
            JsonObject step=new JsonObject();step.addProperty("stage",stage);step.add("actions",new Gson().toJsonTree(actions));step.add("during_tick",new Gson().toJsonTree(inside));step.add("ticks",new Gson().toJsonTree(seen));JsonArray states=new JsonArray();
            for(LivingEntity body:byId.values()){JsonObject state=new JsonObject();state.addProperty("key",key(body));state.addProperty("native_id",body.getId());state.add("pose",pose(body));state.addProperty("remaining_steps",remaining(body));states.add(state);}step.add("states",states);step.add("local_after",pose(minecraft.player));out.add(step);
        }
        JsonObject row=new JsonObject();row.addProperty("seed",seed);row.addProperty("scene",boatMode?"boat":"walking");row.add("steps",out);return row;
    }
    static Player exclusionLocal;
    static JsonArray exclusions(String[] args) throws Exception {
        JsonArray rows=new JsonArray();
        Entity[] entities={new net.minecraft.world.entity.ambient.Bat(EntityType.BAT,client),new net.minecraft.world.entity.decoration.ArmorStand(EntityType.ARMOR_STAND,client),new net.minecraft.world.entity.item.ItemEntity(EntityType.ITEM,client),new net.minecraft.world.entity.projectile.arrow.Arrow(EntityType.ARROW,client)};
        String[] names={"minecraft:bat","minecraft:armor_stand","minecraft:item","minecraft:arrow"};
        Method doPush=LivingEntity.class.getDeclaredMethod(args[27],Entity.class);doPush.setAccessible(true);
        Method pushEntities=LivingEntity.class.getDeclaredMethod(args[28]);pushEntities.setAccessible(true);
        for(int i=0;i<entities.length;i++){
            Entity e=entities[i];e.setPos(.1,65.,-.3);
            JsonObject row=new JsonObject();row.addProperty("type",names[i]);row.addProperty("pushable",e.isPushable());row.addProperty("solid",e.canBeCollidedWith(exclusionLocal));
            exclusionLocal.setPos(.4,65.,-.3);exclusionLocal.setDeltaMovement(.125,-.25,.375);
            row.add("before",pose(exclusionLocal));
            if(e instanceof LivingEntity living){doPush.invoke(living,exclusionLocal);pushEntities.invoke(living);}
            row.add("after",pose(exclusionLocal));rows.add(row);
        }
        return rows;
    }
    public static void main(String[] args){try {
        originalPlayerAiStep=java.lang.invoke.MethodHandles.privateLookupIn(Player.class,java.lang.invoke.MethodHandles.lookup()).findSpecial(Player.class,args[21],java.lang.invoke.MethodType.methodType(void.class),Player.class);
        data=InterpolationHandler.class.getDeclaredField(args[7]);data.setAccessible(true);steps=Class.forName(InterpolationHandler.class.getName()+"$"+args[9]).getDeclaredField(args[8]);steps.setAccessible(true);
        net.minecraft.server.Main.main(new String[]{"--nogui"});MinecraftServer server=null;
        for(int i=0;i<120&&(server==null||server.getTickCount()<20);i++){Thread.sleep(500);if(server==null)server=findServer();}
        if(server==null)throw new IllegalStateException("not ready");MinecraftServer s=server;
        JsonArray result=new JsonArray();s.executeBlocking(()->{for(int x=-16;x<=32;x++)for(int z=-16;z<=16;z++)s.overworld().setBlock(new net.minecraft.core.BlockPos(x,64,z),net.minecraft.world.level.block.Blocks.STONE.defaultBlockState(),0);try{set(s.overworld(),Level.class,args[6],true);try{for(int scene=0;scene<2;scene++){boatMode=scene==1;for(int seed=0;seed<100;seed++)result.add(evaluate(seed,s.overworld(),args));}Files.writeString(Path.of(args[0]+".wolf-teams.json"),new Gson().toJson(wolfTeams())+"\n");Files.writeString(Path.of(args[0]+".wolf.json"),new Gson().toJson(wolfStates(args))+"\n");Files.writeString(Path.of(args[0]+".exclusions.json"),new Gson().toJson(exclusions(args))+"\n");}finally{set(s.overworld(),Level.class,args[6],false);}}catch(Exception e){throw new RuntimeException(e);}});
        Files.writeString(Path.of(args[0]),new Gson().toJson(result)+"\n");s.halt(false);System.exit(0);
    }catch(Throwable e){e.printStackTrace();System.exit(1);}}
}
