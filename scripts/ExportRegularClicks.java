// Original tooling for the unmodified official Java 1.16.1 / 1.21.11 JARs.
// Calls actual menu/slot/item/hash methods; no Minecraft method body is copied.
// The skeletal player/world supplies only inventory and native default feature flags.
// This is a menu primitive/codec oracle, not a network, mode, ownership or recovery test.
import com.google.gson.*;
import io.netty.buffer.Unpooled;
import io.netty.buffer.ByteBuf;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;

public final class ExportRegularClicks {
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
    static JsonObject describe(Object stack) throws Exception {
        var value=new JsonObject();
        if((boolean)isEmpty.invoke(stack)) {value.add("item",JsonNull.INSTANCE);value.addProperty("count",0);}
        else {
            Object item=stackClass.getMethod(legacy?"b":"h").invoke(stack);
            value.addProperty("item",nameOf.invoke(items,item).toString());value.addProperty("count",(int)count.invoke(stack));
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
        Constructor<?> inv=inventoryClass.getConstructors()[0];Object[] args=new Object[inv.getParameterCount()];args[0]=player;inventory=inv.newInstance(args);
        field(player,playerClass,legacy?"bt":"cE",inventory);
        if(!legacy) {
            // Only original getters and native default feature flags are used by ordinary PICKUP.
            // No server/world/player algorithm is replaced by this context provider.
            Object flags=Class.forName("dhb").getField("h").get(null);
            Class<?> data=Class.forName("fnt");
            Object worldData=Proxy.newProxyInstance(data.getClassLoader(),new Class<?>[]{data},(p,m,a)->{
                if(m.getName().equals("J"))return flags;throw new IllegalStateException("unexpected world data access: "+m);
            });
            Object server=allocate(Class.forName("ary"));field(server,Class.forName("net.minecraft.server.MinecraftServer"),"k",worldData);
            Object world=allocate(Class.forName("axf"));field(world,Class.forName("axf"),"H",server);field(player,Class.forName("cgk"),"aU",world);
        }
        mayPlace=slotClass.getMethod("a",stackClass);mayPickup=slotClass.getMethod("a",playerClass);
    }
    public static void main(String[] args) throws Exception {
        init(args[0]);var result=new JsonObject();result.addProperty("version",args[0]);
        var itemProfiles=new JsonArray();Class<?> itemClass=Class.forName(legacy?"bke":"dlp");
        for(var e:byName.entrySet()) {
            Object stack=stack(e.getKey(),1);var item=new JsonObject();item.addProperty("name",e.getKey());item.addProperty("native_id",(int)idOf.invoke(items,e.getValue()));
            item.addProperty("maximum_stack_size",(int)stackClass.getMethod(legacy?"c":"k").invoke(stack));
            item.addProperty("represents_empty",(boolean)isEmpty.invoke(stack));
            boolean plain=true;
            if(!legacy) {
                Method first=e.getValue().getClass().getMethod("a",stackClass,slotClass,Class.forName("dht"),playerClass);
                Method second=e.getValue().getClass().getMethod("a",stackClass,stackClass,slotClass,Class.forName("dht"),playerClass,Class.forName("cic"));
                plain=first.getDeclaringClass()==itemClass&&second.getDeclaringClass()==itemClass;
                item.addProperty("primary_override_owner",first.getDeclaringClass().getName());item.addProperty("secondary_override_owner",second.getDeclaringClass().getName());
                item.addProperty("enabled_by_default_flags",(boolean)stackClass.getMethod("a",Class.forName("dgz")).invoke(stack,Class.forName("dhb").getField("h").get(null)));
            }
            item.addProperty("ordinary_pickup",plain);itemProfiles.add(item);
        }
        result.add("items",itemProfiles);var menuProfiles=new JsonArray();
        var names=new ArrayList<String>();for(int rows=1;rows<=6;rows++)names.add("minecraft:generic_9x"+rows);names.addAll(List.of("minecraft:generic_3x3","minecraft:hopper","minecraft:shulker_box","minecraft:player"));
        for(String name:names) {
            Object menu=menu(name);var record=new JsonObject();record.addProperty("name",name);var slotProfiles=new JsonArray();List<?> slots=slots(menu);
            for(int i=0;i<slots.size();i++) {
                if(name.equals("minecraft:player")&&(i<9||i>44))continue; // crafting/armor/offhand are separate unfinished contracts
                Object slot=slots.get(i);var profile=new JsonObject();profile.addProperty("slot",i);profile.addProperty("native_class",slot.getClass().getName());profile.addProperty("may_pickup",(boolean)mayPickup.invoke(slot,player));
                Field rawIndex=slotClass.getDeclaredField("a");rawIndex.setAccessible(true);
                if(slotClass.getField("c").get(slot)==inventory)profile.addProperty("raw_player_slot",rawIndex.getInt(slot));
                profile.addProperty("base_capacity",(int)slotClass.getMethod("a").invoke(slot));
                var rejected=new JsonArray();for(var entry:byName.entrySet())if(!(boolean)mayPlace.invoke(slot,stack(entry.getKey(),1)))rejected.add(entry.getKey());profile.add("rejected_default_items",rejected);slotProfiles.add(profile);
            }
            record.add("slots",slotProfiles);menuProfiles.add(record);
        }
        result.add("menus",menuProfiles);var cases=new JsonArray();
        String[] sample={"minecraft:stone","minecraft:dirt","minecraft:egg","minecraft:saddle","minecraft:white_shulker_box"};
        for(String menuName:List.of("minecraft:generic_9x3","minecraft:shulker_box","minecraft:player"))for(int index:menuName.equals("minecraft:player")?new int[]{9,36,44}:new int[]{0,27,54})
            for(String source:sample)for(String carried:sample) {
                int sourceMax=(int)stackClass.getMethod(legacy?"c":"k").invoke(stack(source,1));int cursorMax=(int)stackClass.getMethod(legacy?"c":"k").invoke(stack(carried,1));
                int[] sourceCounts={0,1,2,3,sourceMax/2,sourceMax-1,sourceMax,sourceMax+1};int[] cursorCounts={0,1,2,3,cursorMax/2,cursorMax-1,cursorMax,cursorMax+1};
                for(int sc:Arrays.stream(sourceCounts).distinct().filter(c->c>=0).toArray())for(int cc:Arrays.stream(cursorCounts).distinct().filter(c->c>=0).toArray())for(int button=0;button<2;button++) {
                    Object menu=menu(menuName);Object slot=slots(menu).get(index);Object original=stack(source,sc),held=stack(carried,cc);
                    JsonObject requestedSource=describe(original),requestedCursor=describe(held);
                    slotSet(slot,original);cursor(menu,held);
                    var c=new JsonObject();c.addProperty("menu",menuName);c.addProperty("slot",index);c.addProperty("button",button);c.add("requested_source",requestedSource);c.add("requested_cursor",requestedCursor);
                    c.add("source_before",describe(slotGet(slot)));c.add("cursor_before",describe(cursor(menu)));
                    c.addProperty("valid_requested_counts",sc<=sourceMax&&cc<=cursorMax);
                    c.addProperty("valid_default_counts",(int)count.invoke(slotGet(slot))<=sourceMax&&(int)count.invoke(cursor(menu))<=cursorMax);c.addProperty("slot_allows_cursor",(boolean)mayPlace.invoke(slot,held));
                    Object returned=menuClass.getMethod("a",int.class,int.class,Class.forName(legacy?"bgq":"dhu"),playerClass).invoke(menu,index,button,pickup,player);
                    c.add("source_after",describe(slotGet(slot)));c.add("cursor_after",describe(cursor(menu)));if(legacy)c.add("legacy_returned",describe(returned));
                    cases.add(c);
                }
            }
        result.add("cases",cases);
        var swaps=new JsonArray();Object swap=Class.forName(legacy?"bgq":"dhu").getField("c").get(null);
        for(String menuName:List.of("minecraft:generic_9x3","minecraft:shulker_box","minecraft:player"))for(int index:menuName.equals("minecraft:player")?new int[]{9,35}:new int[]{0,27})
            for(String source:sample)for(String carried:sample) {
                int sourceMax=(int)stackClass.getMethod(legacy?"c":"k").invoke(stack(source,1));int hotbarMax=(int)stackClass.getMethod(legacy?"c":"k").invoke(stack(carried,1));
                for(int sc:Arrays.stream(new int[]{0,1,2,3,sourceMax/2,sourceMax-1,sourceMax}).distinct().filter(c->c>=0).toArray())
                    for(int hc:Arrays.stream(new int[]{0,1,2,3,hotbarMax/2,hotbarMax-1,hotbarMax}).distinct().filter(c->c>=0&&c<=hotbarMax).toArray())for(int hotbar:new int[]{0,8}) {
                        if(sc>sourceMax)continue;
                        Object menu=menu(menuName);Object slot=slots(menu).get(index);Object original=stack(source,sc),held=stack(carried,hc);
                        slotSet(slot,original);inventoryClass.getMethod("a",int.class,stackClass).invoke(inventory,hotbar,held);cursor(menu,empty);
                        var c=new JsonObject();c.addProperty("menu",menuName);c.addProperty("slot",index);c.addProperty("hotbar",hotbar);c.add("source_before",describe(original));c.add("hotbar_before",describe(held));
                        c.addProperty("slot_allows_hotbar",(boolean)mayPlace.invoke(slot,held));
                        Object returned=menuClass.getMethod("a",int.class,int.class,Class.forName(legacy?"bgq":"dhu"),playerClass).invoke(menu,index,hotbar,swap,player);
                        c.add("source_after",describe(slotGet(slot)));c.add("hotbar_after",describe(inventoryClass.getMethod("a",int.class).invoke(inventory,hotbar)));if(legacy)c.add("legacy_returned",describe(returned));swaps.add(c);
                    }
            }
        result.add("swaps",swaps);
        {
            var packets=new JsonArray();
            for(String item:sample)for(int amount:new int[]{0,1,3,16})for(int window:legacy?new int[]{0,3,127}:new int[]{0,3,128})for(int button=0;button<2;button++) {
                Object predecessor=stack(item,amount);var p=new JsonObject();p.addProperty("window",window);p.addProperty("revision",32767);p.addProperty("slot",window==0?9:0);p.addProperty("button",button);p.add("cursor_comparison",describe(predecessor));
                p.addProperty("payload_hex",HexFormat.of().formatHex(legacy?legacyPacket(predecessor,window,32767,window==0?9:0,button):modernPacket(predecessor,window,32767,window==0?9:0,button)));packets.add(p);
            }
            result.add(legacy?"legacy_packets":"modern_packets",packets);
        }
        Files.writeString(Path.of(args[1]),new GsonBuilder().serializeNulls().setPrettyPrinting().create().toJson(result)+"\n");
        System.out.println(args[0]+": "+itemProfiles.size()+" item profiles, "+menuProfiles.size()+" native slot layouts, "+cases.size()+" original PICKUP and "+swaps.size()+" SWAP clicks verified");
        if(legacy)Class.forName("v").getMethod("h").invoke(null);
    }
}
