// Own calls to original furnace QUICK_MOVE, recipes, fuel tags and packet codec.
// No game method body is replaced. The constructor's SimpleContainer has no XP
// recipe-use ledger; real smelting and XP/world effects need separate live checks.
import com.google.gson.*;
import com.mojang.serialization.*;
import java.lang.reflect.*;
import io.netty.buffer.*;
import java.nio.file.*;
import java.util.*;
public final class ExportFurnaceTransfers {
    static Object recipes;
    static void context() throws Exception {
        ExportItemProperties.init("1.21.11");
        ExportCraftingReturns.old = false;
        for (Object type : (Iterable<?>)ExportItemProperties.components)
            ExportCraftingReturns.types.put(ExportItemProperties.name.invoke(ExportItemProperties.components,type).toString(),type);
        ExportCraftingReturns.ops = (DynamicOps<JsonElement>)Class.forName("ams")
            .getMethod("a",DynamicOps.class,Class.forName("jf$a"))
            .invoke(null,JsonOps.INSTANCE,ExportItemComponents.registries);
        Object world=ExportInventoryTransfers.playerClass.getMethod("ao").invoke(ExportInventoryTransfers.player);
        Object server=Class.forName("axf").getMethod("s").invoke(world);
        Object scoreboard=Class.forName("anm").getConstructor(Class.forName("net.minecraft.server.MinecraftServer")).newInstance(server);
        ExportInventoryTransfers.field(server,Class.forName("net.minecraft.server.MinecraftServer"),"at",scoreboard);
        recipes=Class.forName("dqz").getConstructor(Class.forName("jf$a")).newInstance(ExportItemComponents.registries);
        Object profiler=Class.forName("bzi").getField("a").get(null);
        Method prepare=Class.forName("dqz").getDeclaredMethod("a",Class.forName("baz"),Class.forName("bzm"));prepare.setAccessible(true);
        Object map=prepare.invoke(recipes,ExportItemComponents.resourceManager,profiler);
        Method apply=Class.forName("dqz").getDeclaredMethod("a",Class.forName("dra"),Class.forName("baz"),Class.forName("bzm"));apply.setAccessible(true);
        apply.invoke(recipes,map,ExportItemComponents.resourceManager,profiler);
        Object flags=Class.forName("dhb").getField("h").get(null);
        Class.forName("dqz").getMethod("a",Class.forName("dgz")).invoke(recipes,flags);
        Object stats=Class.forName("bdg").getConstructor(Class.forName("net.minecraft.server.MinecraftServer"),Path.class)
            .newInstance(server,Path.of("unused-oracle-stats.json"));
        ExportInventoryTransfers.field(ExportInventoryTransfers.player,Class.forName("axg"),"cN",stats);
        Object resources=ExportInventoryTransfers.allocate(Class.forName("ane"));
        ExportInventoryTransfers.field(resources,Class.forName("ane"),"e",recipes);
        Constructor<?> c=Class.forName("net.minecraft.server.MinecraftServer$a").getDeclaredConstructors()[0];c.setAccessible(true);
        ExportInventoryTransfers.field(server,Class.forName("net.minecraft.server.MinecraftServer"),"aD",c.newInstance(null,resources));
        Object fuels=Class.forName("emb").getMethod("a",Class.forName("jf$a"),Class.forName("dgz"))
            .invoke(null,ExportItemComponents.registries,flags);
        ExportInventoryTransfers.field(server,Class.forName("net.minecraft.server.MinecraftServer"),"aJ",fuels);
    }
    static Object menu(String name) throws Exception {
        return Class.forName(switch(name){case "furnace"->"dih";case "blast_furnace"->"dhp";case "smoker"->"djm";default->throw new IllegalArgumentException(name);})
            .getConstructor(int.class,ExportInventoryTransfers.inventoryClass).newInstance(3,ExportInventoryTransfers.inventory);
    }
    static byte[] packet(Object packet) throws Exception {
        ByteBuf bytes=Unpooled.buffer();
        try {
            Object wire=ExportItemComponents.buffer(bytes),codec=Class.forName("ahw").getField("a").get(null);
            Class.forName("aao").getMethod("encode",Object.class,Object.class).invoke(codec,wire,packet);
            byte[] raw=new byte[bytes.readableBytes()];bytes.getBytes(0,raw);
            Object decoded=Class.forName("aao").getMethod("decode",Object.class).invoke(codec,wire);
            if(bytes.isReadable())throw new IllegalStateException("unconsumed UPDATE_RECIPES bytes");
            // Property-set collections may re-encode in a different order.
            Map<?,?> before=(Map<?,?>)Class.forName("ahw").getMethod("b").invoke(packet);
            Map<?,?> after=(Map<?,?>)Class.forName("ahw").getMethod("b").invoke(decoded);
            if(!before.keySet().equals(after.keySet()))throw new IllegalStateException("UPDATE_RECIPES keys changed");
            Field members=Class.forName("drb").getDeclaredField("k");members.setAccessible(true);
            for(Object key:before.keySet())if(!members.get(before.get(key)).equals(members.get(after.get(key))))
                throw new IllegalStateException("UPDATE_RECIPES property membership changed");
            return raw;
        } finally {bytes.release();}
    }
    public static void main(String[] args) throws Exception {
        ExportInventoryTransfers.init("1.21.11");context();
        JsonObject out=new JsonObject();out.addProperty("version","1.21.11");
        Object properties=Class.forName("dqz").getMethod("b").invoke(recipes);
        // Stable wire fixture ordering of the same native set memberships.
        // Set.copyOf iteration is JVM-randomized; no membership or rule changes.
        Map<?,?> original=(Map<?,?>)properties;
        Map<String,Object> keys=new TreeMap<>();
        for(Object key:original.keySet())keys.put(Class.forName("amt").getMethod("a").invoke(key).toString(),key);
        Map<Object,Object> ordered=new LinkedHashMap<>();
        Field memberField=Class.forName("drb").getDeclaredField("k");memberField.setAccessible(true);
        for(Object key:keys.values()) {
            Object property=original.get(key);
            Map<Integer,Object> members=new TreeMap<>();
            for(Object holder:(Set<?>)memberField.get(property)) {
                Object item=Class.forName("jd").getMethod("a").invoke(holder);
                members.put((Integer)ExportInventoryTransfers.idOf.invoke(ExportInventoryTransfers.items,item),holder);
            }
            memberField.set(property,new LinkedHashSet<>(members.values()));ordered.put(key,property);
        }
        properties=ordered;
        Object cutters=Class.forName("dqz").getMethod("c").invoke(recipes);
        Object packet=Class.forName("ahw").getConstructors()[0].newInstance(properties,cutters);
        out.addProperty("properties_packet_hex",ExportItemComponents.hex(packet(packet)));
        JsonObject sets=new JsonObject();
        for (var e:((Map<?,?>)properties).entrySet()) {
            JsonArray members=new JsonArray();
            Field f=Class.forName("drb").getDeclaredField("k");f.setAccessible(true);
            List<Integer> ids=new ArrayList<>();
            for(Object holder:(Set<?>)f.get(e.getValue())) {
                Object item=Class.forName("jd").getMethod("a").invoke(holder);
                ids.add((Integer)ExportInventoryTransfers.idOf.invoke(ExportInventoryTransfers.items,item));
            }
            Collections.sort(ids);for(int id:ids)members.add(id);
            sets.add(Class.forName("amt").getMethod("a").invoke(e.getKey()).toString(),members);
        }
        out.add("properties",sets);
        JsonArray cases=new JsonArray();
        for(JsonElement e:new JsonParser().parse(Files.readString(Path.of(args[0]))).getAsJsonArray()) {
            JsonObject r=e.getAsJsonObject();ExportInventoryTransfers.reset();Object menu=menu(r.get("menu").getAsString());
            int source=r.get("slot").getAsInt();String fixture=r.get("fixture").getAsString(),item=r.get("item").getAsString();
            String data=r.get("data").getAsBoolean()?"furnace":null;
            Object stack=ExportCraftingInputTransfers.stack(item,7,data);
            int max=(Integer)ExportItemProperties.stack.getMethod("k").invoke(stack);
            stack=ExportCraftingInputTransfers.stack(item,Math.min(7,max),data);
            List<?> slots=ExportInventoryTransfers.slots(menu);
            if(!fixture.equals("empty"))for(int i=0;i<39;i++)ExportInventoryTransfers.slotSet(slots.get(i),ExportInventoryTransfers.stack("minecraft:dirt",64));
            int destination=source<3?(source==2?38:3):0;
            Method smelt=Class.forName("dhk").getDeclaredMethod("b",ExportInventoryTransfers.stackClass);smelt.setAccessible(true);
            Method fuel=Class.forName("dhk").getDeclaredMethod("c",ExportInventoryTransfers.stackClass);fuel.setAccessible(true);
            boolean canSmelt=(Boolean)smelt.invoke(menu,stack),isFuel=(Boolean)fuel.invoke(menu,stack);
            if(source>=3)destination=canSmelt?0:isFuel?1:source<30?30:3;
            if(fixture.equals("partial"))ExportInventoryTransfers.slotSet(slots.get(destination),ExportCraftingInputTransfers.stack(item,Math.max(1,max-2),data));
            if(fixture.equals("merge-late")) {
                for(int i=0;i<39;i++)ExportInventoryTransfers.slotSet(slots.get(i),ExportInventoryTransfers.empty);
                if(source<3)ExportInventoryTransfers.slotSet(slots.get(source==2?3:38),ExportCraftingInputTransfers.stack(item,Math.max(1,max-2),data));
            }
            if(fixture.equals("bucket-fuel"))ExportInventoryTransfers.slotSet(slots.get(1),ExportInventoryTransfers.stack("minecraft:bucket",1));
            ExportInventoryTransfers.slotSet(slots.get(source),stack);
            ExportInventoryTransfers.cursor(menu,r.get("held").getAsBoolean()?ExportCraftingInputTransfers.stack("minecraft:diamond_helmet",1,"cursor"):ExportInventoryTransfers.empty);
            Object gameMode=ExportInventoryTransfers.player.getClass().getField("h").get(ExportInventoryTransfers.player);
            ExportInventoryTransfers.field(gameMode,Class.forName("axh"),"e",Class.forName("dwl").getField(r.get("mode").getAsString().equals("creative")?"b":"a").get(null));
            JsonObject row=new JsonObject();row.add("request",r);row.addProperty("can_smelt",canSmelt);row.addProperty("is_fuel",isFuel);
            row.add("before",ExportCraftingInputTransfers.snapshot(menu));
            row.addProperty("cursor_before_hex",ExportItemProperties.encoded(ExportInventoryTransfers.cursor(menu)));
            ExportInventoryTransfers.quickMove(menu,source);
            row.add("after",ExportCraftingInputTransfers.snapshot(menu));
            row.addProperty("cursor_after_hex",ExportItemProperties.encoded(ExportInventoryTransfers.cursor(menu)));cases.add(row);
        }
        out.add("cases",cases);Files.writeString(Path.of(args[1]),out.toString());
        ((AutoCloseable)ExportItemComponents.resourceManager).close();
    }
}
