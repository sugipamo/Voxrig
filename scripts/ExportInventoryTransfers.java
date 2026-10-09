// Original tooling for the unmodified official Java 1.16.1 / 1.21.11 JARs.
// Calls actual menu/slot/item/hash methods; no Minecraft method body is copied.
// The skeletal player/world supplies only inventory, native equipment and default feature flags.
// This is a menu primitive/codec oracle, not a network, mode, ownership or recovery test.
import com.google.gson.*;
import io.netty.buffer.Unpooled;
import io.netty.buffer.ByteBuf;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;

public final class ExportInventoryTransfers {
    static boolean legacy;
    static Class<?> stackClass, playerClass, inventoryClass, menuClass, slotClass, registryClass;
    static Object items, menus, empty, pickup, player, inventory;
    static Map<String,Object> byName = new TreeMap<>();
    static Method nameOf, idOf, count, isEmpty, mayPlace, mayPickup;
    static Object allocate(Class<?> cls) throws Exception {
        Class<?> unsafeClass=Class.forName("sun.misc.Unsafe");
        Field f=unsafeClass.getDeclaredField("theUnsafe");f.setAccessible(true);
        return unsafeClass.getMethod("allocateInstance",Class.class).invoke(f.get(null),cls);
    }
    static void field(Object obj, Class<?> owner, String name, Object value) throws Exception {
        Field f=owner.getDeclaredField(name);f.setAccessible(true);f.set(obj,value);
    }
    static Object stack(String name,int amount) throws Exception {
        if(amount==0)return empty;
        return stackClass.getConstructor(Class.forName(legacy?"bqa":"dwn"),int.class).newInstance(byName.get(name),amount);
    }
    static byte[] legacyNbt(Object stack) throws Exception {
        Object tag=stackClass.getMethod("o").invoke(stack);
        Object buffer=Class.forName("mg").getConstructor(ByteBuf.class).newInstance(Unpooled.buffer());
        try {
            Class.forName("mg").getMethod("a",Class.forName("le")).invoke(buffer,tag);
            byte[] bytes=new byte[((ByteBuf)buffer).readableBytes()];((ByteBuf)buffer).getBytes(0,bytes);return bytes;
        } finally {((ByteBuf)buffer).release();}
    }
    static JsonObject describe(Object stack) throws Exception {
        var value=new JsonObject();
        if((boolean)isEmpty.invoke(stack)) {value.add("item",JsonNull.INSTANCE);value.addProperty("count",0);}
        else {
            Object item=stackClass.getMethod(legacy?"b":"h").invoke(stack);
            value.addProperty("item",nameOf.invoke(items,item).toString());value.addProperty("count",(int)count.invoke(stack));
            if(legacy) {var nbt=new JsonArray();for(byte b:legacyNbt(stack))nbt.add(Byte.toUnsignedInt(b));value.add("nbt",nbt);}
        }
        return value;
    }
    static Object cursor(Object menu) throws Exception {
        return legacy?inventoryClass.getMethod("m").invoke(inventory):menuClass.getMethod("g").invoke(menu);
    }
    static void cursor(Object menu,Object stack) throws Exception {
        if(legacy)inventoryClass.getMethod("g",stackClass).invoke(inventory,stack);
        else menuClass.getMethod("a",stackClass).invoke(menu,stack);
    }
    static Object menu(String name) throws Exception {
        if(name.equals("minecraft:player"))return Class.forName(legacy?"bhf":"dim").getConstructor(inventoryClass,boolean.class,playerClass).newInstance(inventory,false,player);
        Object type=null;
        for(Object candidate:(Iterable<?>)menus)if(nameOf.invoke(menus,candidate).toString().equals(name))type=candidate;
        if(type==null)throw new IllegalStateException("menu not registered: "+name);
        int rows=name.startsWith("minecraft:generic_9x")?Integer.parseInt(name.substring(name.length()-1)):0;
        int size=rows>0?rows*9:name.equals("minecraft:hopper")?5:name.equals("minecraft:generic_3x3")?9:27;
        String nativeClass=rows>0?(legacy?"bgp":"dhs"):name.equals("minecraft:generic_3x3")?(legacy?"bgx":"die"):name.equals("minecraft:hopper")?(legacy?"bhd":"dik"):(legacy?"bht":"djf");
        Object container=Class.forName(legacy?"anm":"cdk").getConstructor(int.class).newInstance(size);
        Constructor<?> factory=Arrays.stream(Class.forName(nativeClass).getDeclaredConstructors()).filter(c->c.getParameterCount()==(rows>0?5:3)).findFirst().orElseThrow();
        factory.setAccessible(true);
        return rows>0?factory.newInstance(type,3,inventory,container,rows):factory.newInstance(3,inventory,container);
    }
    static List<?> slots(Object menu) throws Exception {return (List<?>)menuClass.getField(legacy?"a":"k").get(menu);}
    static Object slotGet(Object slot) throws Exception {return slotClass.getMethod(legacy?"e":"g").invoke(slot);}
    static void slotSet(Object slot,Object stack) throws Exception {slotClass.getMethod(legacy?"d":"f",stackClass).invoke(slot,stack);}
    static byte[] legacyPacket(Object comparison,int window,int action,int slot,int button) throws Exception {
        Class<?> packetClass=Class.forName("rj"),bufferClass=Class.forName("mg");
        // The dedicated server JAR strips the client-only constructor. Populate
        // its unchanged packet fields, then call the actual codec in both directions.
        Object packet=packetClass.getConstructor().newInstance();
        field(packet,packetClass,"a",window);field(packet,packetClass,"b",slot);field(packet,packetClass,"c",button);
        field(packet,packetClass,"d",(short)action);field(packet,packetClass,"e",comparison);field(packet,packetClass,"f",pickup);
        Object buffer=bufferClass.getConstructor(ByteBuf.class).newInstance(Unpooled.buffer());
        try {
            packetClass.getMethod("b",bufferClass).invoke(packet,buffer);
            var bytes=new byte[((ByteBuf)buffer).readableBytes()];((ByteBuf)buffer).getBytes(0,bytes);
            Object decoded=packetClass.getConstructor().newInstance();packetClass.getMethod("a",bufferClass).invoke(decoded,buffer);
            if((int)packetClass.getMethod("b").invoke(decoded)!=window || (short)packetClass.getMethod("e").invoke(decoded)!=(short)action
                || (int)packetClass.getMethod("c").invoke(decoded)!=slot || (int)packetClass.getMethod("d").invoke(decoded)!=button
                || packetClass.getMethod("g").invoke(decoded)!=pickup || !describe(packetClass.getMethod("f").invoke(decoded)).equals(describe(comparison))
                || ((ByteBuf)buffer).isReadable())throw new IllegalStateException("native legacy click codec roundtrip");
            return bytes;
        } finally {((ByteBuf)buffer).release();}
    }
    static byte[] modernPacket(Object predecessor,int window,int revision,int slot,int button) throws Exception {
        Class<?> hashClass=Class.forName("xa");
        Class<?> generator=Class.forName("wz$a");
        Object rejectComponents=Proxy.newProxyInstance(generator.getClassLoader(),new Class<?>[]{generator},(p,m,a)->{throw new IllegalStateException("default stack unexpectedly requested component hash: "+m);});
        Object hash=hashClass.getMethod("b",stackClass,generator).invoke(null,predecessor,rejectComponents);
        Object map=Class.forName("it.unimi.dsi.fastutil.ints.Int2ObjectMaps").getMethod("emptyMap").invoke(null);
        Class<?> packetClass=Class.forName("ais");
        Object packet=packetClass.getConstructors()[0].newInstance(window,revision,(short)slot,(byte)button,pickup,map,hash);
        Object registries=Class.forName("jr").getMethod("a",registryClass).invoke(null,Class.forName("mi").getField("aR").get(null));
        Object buffer=Class.forName("xq").getConstructor(ByteBuf.class,Class.forName("jr")).newInstance(Unpooled.buffer(),registries);
        Object codec=packetClass.getField("a").get(null);
        Class<?> codecClass=Class.forName("aao");
        try {
            codecClass.getMethod("encode",Object.class,Object.class).invoke(codec,buffer,packet);
            var bytes=new byte[((ByteBuf)buffer).readableBytes()];((ByteBuf)buffer).getBytes(0,bytes);
            Object decoded=codecClass.getMethod("decode",Object.class).invoke(codec,buffer);
            if((int)packetClass.getMethod("b").invoke(decoded)!=window || (int)packetClass.getMethod("e").invoke(decoded)!=revision
                || (short)packetClass.getMethod("f").invoke(decoded)!=(short)slot || (byte)packetClass.getMethod("g").invoke(decoded)!=(byte)button
                || packetClass.getMethod("h").invoke(decoded)!=pickup || !packetClass.getMethod("i").invoke(decoded).equals(map)
                || !packetClass.getMethod("j").invoke(decoded).equals(hash) || ((ByteBuf)buffer).isReadable())throw new IllegalStateException("native click codec roundtrip");
            return bytes;
        } finally {((ByteBuf)buffer).release();}
    }
    static void init(String version) throws Exception {
        legacy=version.equals("1.16.1");if(!legacy&&!version.equals("1.21.11"))throw new IllegalArgumentException("version");
        if(!legacy)Class.forName("w").getMethod("a").invoke(null);
        Object gameVersion=Class.forName(legacy?"u":"w").getMethod(legacy?"a":"b").invoke(null);
        String actual=(String)Class.forName(legacy?"com.mojang.bridge.game.GameVersion":"aa").getMethod(legacy?"getName":"c").invoke(gameVersion);
        if(!actual.equals(version))throw new IllegalStateException("version mismatch");
        Class.forName(legacy?"uj":"amv").getMethod("a").invoke(null);
        registryClass=Class.forName(legacy?"gl":"jq");nameOf=registryClass.getMethod("b",Object.class);idOf=registryClass.getMethod("a",Object.class);
        Class<?> builtins=Class.forName(legacy?"gl":"mi");items=builtins.getField(legacy?"am":"h").get(null);menus=builtins.getField(legacy?"aM":"q").get(null);
        for(Object item:(Iterable<?>)items)byName.put(nameOf.invoke(items,item).toString(),item);
        stackClass=Class.forName(legacy?"bki":"dlt");playerClass=Class.forName(legacy?"bec":"ddm");inventoryClass=Class.forName(legacy?"beb":"ddl");menuClass=Class.forName(legacy?"bgi":"dhi");slotClass=Class.forName(legacy?"bhw":"dji");
        empty=stackClass.getField(legacy?"b":"l").get(null);count=stackClass.getMethod(legacy?"E":"N");isEmpty=stackClass.getMethod(legacy?"a":"f");
        pickup=Class.forName(legacy?"bgq":"dhu").getField("a").get(null);
        player=allocate(Class.forName(legacy?"ze":"axg"));
        field(player,Class.forName(legacy?"aom":"cgk"),legacy?"R":"ay",true);
        field(player,Class.forName(legacy?"aom":"cgk"),legacy?"f":"aP",Class.forName(legacy?"aoq":"cgu").getField(legacy?"bb":"cb").get(null));
        Constructor<?> inv=inventoryClass.getConstructors()[0];Object[] args=new Object[inv.getParameterCount()];args[0]=player;
        if(!legacy) {
            Object equipment=Class.forName("cgo").getConstructor().newInstance();
            args[1]=equipment;field(player,Class.forName("chl"),"cb",equipment);
        }
        inventory=inv.newInstance(args);
        field(player,playerClass,legacy?"bt":"cE",inventory);
        if(!legacy) {
            // Only original getters and native default feature flags are used by ordinary native menus.
            // No server/world/player algorithm is replaced by this context provider.
            Object flags=Class.forName("dhb").getField("h").get(null);
            Class<?> data=Class.forName("fnt");
            Object worldData=Proxy.newProxyInstance(data.getClassLoader(),new Class<?>[]{data},(p,m,a)->{
                if(m.getName().equals("J"))return flags;throw new IllegalStateException("unexpected world data access: "+m);
            });
            Object server=allocate(Class.forName("ary"));field(server,Class.forName("net.minecraft.server.MinecraftServer"),"k",worldData);
            Object world=allocate(Class.forName("axf"));field(world,Class.forName("axf"),"H",server);field(player,Class.forName("cgk"),"aU",world);
            field(player,Class.forName("axg"),"h",Class.forName("axh").getConstructor(Class.forName("axg")).newInstance(player));
        }
        if(legacy) {
            Object world=allocate(Class.forName("zd"));field(player,Class.forName("aom"),"l",world);
            Object gameMode=Class.forName("zf").getConstructor(Class.forName("zd")).newInstance(world);
            field(gameMode,Class.forName("zf"),"b",player);field(player,Class.forName("ze"),"d",gameMode);
        }
        mayPlace=slotClass.getMethod("a",stackClass);mayPickup=slotClass.getMethod("a",playerClass);
    }
    static JsonArray snapshot(Object menu) throws Exception {
        var out=new JsonArray(); for(Object slot:slots(menu))out.add(describe(slotGet(slot)));return out;
    }
    static void reset() throws Exception {
        Constructor<?> inv=inventoryClass.getConstructors()[0];Object[] args=new Object[inv.getParameterCount()];args[0]=player;
        if(!legacy){Object equipment=Class.forName("cgo").getConstructor().newInstance();args[1]=equipment;field(player,Class.forName("chl"),"cb",equipment);}
        inventory=inv.newInstance(args);field(player,playerClass,legacy?"bt":"cE",inventory);
    }
    static Object quickMove(Object menu,int index) throws Exception {
        pickup=Class.forName(legacy?"bgq":"dhu").getField("b").get(null);
        return menuClass.getMethod("a",int.class,int.class,Class.forName(legacy?"bgq":"dhu"),playerClass).invoke(menu,index,0,pickup,player);
    }
    static JsonObject move(String name,int index,String item,int amount,String fixture) throws Exception {
        reset();Object menu=menu(name);List<?> slots=slots(menu);Object sourceSlot=slots.get(index);
        // All destination fixtures are real native setter results, not rounded requests.
        for(int i=0;i<slots.size();i++) {
            if(i==index || (name.equals("minecraft:player")&&(i<9||i>44)))continue;
            String fill=fixture.equals("blocked")?"minecraft:dirt":fixture.equals("merge")?item:null;
            if(fill!=null) {
                int max=(int)stackClass.getMethod(legacy?"c":"k").invoke(stack(fill,1));
                slotSet(slots.get(i),stack(fill,fixture.equals("merge")?Math.max(1,max-1):max));
            }
        }
        if(fixture.equals("equipment_occupied")) {
            for(int i:new int[]{5,6,7,8,45})slotSet(slots.get(i),stack("minecraft:stone",1));
        }
        Object requested=stack(item,amount);JsonObject requestedValue=describe(requested);
        slotSet(sourceSlot,requested);cursor(menu,empty);
        var result=new JsonObject();result.addProperty("menu",name);result.addProperty("slot",index);result.addProperty("fixture",fixture);
        result.add("requested_source",requestedValue);result.add("before",snapshot(menu));result.add("cursor_before",describe(cursor(menu)));
        Object returned=quickMove(menu,index);
        result.add("after",snapshot(menu));result.add("cursor_after",describe(cursor(menu)));if(legacy)result.add("legacy_returned",describe(returned));
        return result;
    }
    public static void main(String[] args) throws Exception {
        init(args[0]);var result=new JsonObject();result.addProperty("version",args[0]);var moves=new JsonArray();var routes=new JsonArray();
        for(var item:byName.entrySet()) {
            if((boolean)isEmpty.invoke(stack(item.getKey(),1)))continue;
            var route=new JsonObject();route.addProperty("name",item.getKey());route.addProperty("native_id",(int)idOf.invoke(items,item.getValue()));
            if(legacy) {var nbt=new JsonArray();for(byte b:legacyNbt(stack(item.getKey(),1)))nbt.add(Byte.toUnsignedInt(b));route.add("default_legacy_nbt",nbt);}
            Integer preferred=null;
            for(int index:new int[]{9,36}) {
                JsonObject c=move("minecraft:player",index,item.getKey(),1,"empty");moves.add(c);
                JsonArray after=c.getAsJsonArray("after");int actual=-1;
                for(int i=0;i<after.size();i++)if(after.get(i).getAsJsonObject().get("count").getAsInt()>0) {
                    if(actual!=-1)throw new IllegalStateException("single item moved to multiple slots");actual=i;
                }
                if(actual>=5&&actual<=8||actual==45) {
                    if(preferred!=null&&preferred!=actual)throw new IllegalStateException("equipment route differs by source");preferred=actual;
                } else if(actual!=(index==9?36:9))throw new IllegalStateException("unexpected native empty route: "+item.getKey()+" "+actual);
            }
            if(preferred==null)route.add("preferred_equipment_slot",JsonNull.INSTANCE);else route.addProperty("preferred_equipment_slot",preferred);
            routes.add(route);
        }
        var samples=List.of("minecraft:stone","minecraft:dirt","minecraft:egg","minecraft:saddle","minecraft:white_shulker_box","minecraft:diamond_helmet","minecraft:shield","minecraft:carved_pumpkin","minecraft:bundle");
        var names=new ArrayList<String>();for(int rows=1;rows<=6;rows++)names.add("minecraft:generic_9x"+rows);names.addAll(List.of("minecraft:generic_3x3","minecraft:hopper","minecraft:shulker_box","minecraft:player"));
        for(String name:names) {
            int storage=name.startsWith("minecraft:generic_9x")?Integer.parseInt(name.substring(name.length()-1))*9:name.equals("minecraft:hopper")?5:name.equals("minecraft:generic_3x3")?9:27;
            int[] indices=name.equals("minecraft:player")?new int[]{5,8,9,35,36,44,45}:new int[]{0,storage-1,storage,storage+26,storage+27,storage+35};
            for(String sample:samples) {
                if(!byName.containsKey(sample))continue;
                int max=(int)stackClass.getMethod(legacy?"c":"k").invoke(stack(sample,1));
                for(int index:indices)for(int amount:Arrays.stream(new int[]{1,Math.min(3,max),max}).distinct().toArray())
                    for(String fixture:name.equals("minecraft:player")?List.of("empty","merge","blocked","equipment_occupied"):List.of("empty","merge","blocked"))
                        moves.add(move(name,index,sample,amount,fixture));
            }
        }
        result.add("routes",routes);result.add("cases",moves);
        // Native armor/offhand acceptance and capacity, queried on actual empty equipment.
        reset();Object playerMenu=menu("minecraft:player");var equipmentSlots=new JsonArray();
        for(int index:new int[]{5,6,7,8,45}) {
            Object slot=slots(playerMenu).get(index);var p=new JsonObject();p.addProperty("slot",index);p.addProperty("native_class",slot.getClass().getName());
            p.addProperty("base_capacity",(int)slotClass.getMethod("a").invoke(slot));p.addProperty("may_pickup",(boolean)mayPickup.invoke(slot,player));
            var accepted=new JsonArray();for(String item:byName.keySet())if((boolean)mayPlace.invoke(slot,stack(item,1)))accepted.add(item);
            p.add("accepted_default_items",accepted);equipmentSlots.add(p);
        }
        result.add("equipment_slots",equipmentSlots);
        var packets=new JsonArray();
        for(String item:List.of("minecraft:stone","minecraft:carved_pumpkin","minecraft:shield"))for(int amount:new int[]{0,1})for(int window:legacy?new int[]{0,3,127}:new int[]{0,3,128}) {
            pickup=Class.forName(legacy?"bgq":"dhu").getField("b").get(null);
            Object comparison=stack(item,amount);var packet=new JsonObject();packet.addProperty("window",window);packet.addProperty("revision",32767);packet.addProperty("slot",window==0?9:0);packet.addProperty("button",0);
            packet.add("comparison",describe(comparison));packet.addProperty("payload_hex",HexFormat.of().formatHex(legacy?legacyPacket(comparison,window,32767,window==0?9:0,0):modernPacket(comparison,window,32767,window==0?9:0,0)));packets.add(packet);
        }
        result.add("packets",packets);
        Files.writeString(Path.of(args[1]),new GsonBuilder().serializeNulls().setPrettyPrinting().create().toJson(result)+"\n");
        System.out.println(args[0]+": "+routes.size()+" default item routes, "+moves.size()+" original QUICK_MOVE cases, "+packets.size()+" native codec roundtrips");
        if(legacy)Class.forName("v").getMethod("h").invoke(null);
    }
}
