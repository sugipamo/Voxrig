// Observe unchanged furnace constructors and slot rules on pinned official JARs.
// Actual vanilla tag loaders and FuelValues supply fuel context. No smelting,
// output-take callback, XP, world tick or replacement native method is executed.
import com.google.gson.*;
import java.io.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import java.util.zip.*;

public final class ExportFurnaceMenus {
    static JsonObject tags = new JsonObject();
    static void loadLegacyTags(String jar) throws Exception {
        Object manager=Class.forName("adh").getConstructor().newInstance();
        Object collection=Class.forName("adh").getMethod("b").invoke(manager);
        Map<Object,Object> builders=new HashMap<>();
        try(ZipFile zip=new ZipFile(jar)) {
            for(ZipEntry e:Collections.list(zip.entries())) {
                String name=e.getName(),prefix="data/minecraft/tags/items/";
                if(!name.startsWith(prefix)||!name.endsWith(".json"))continue;
                Object key=Class.forName("uh").getConstructor(String.class).newInstance("minecraft:"+name.substring(prefix.length(),name.length()-5));
                Object builder=Class.forName("adf$a").getMethod("a").invoke(null);
                try(Reader r=new InputStreamReader(zip.getInputStream(e),java.nio.charset.StandardCharsets.UTF_8)) {
                    Class.forName("adf$a").getMethod("a",JsonObject.class,String.class).invoke(builder,new JsonParser().parse(r).getAsJsonObject(),"vanilla");
                }
                builders.put(key,builder);
            }
        }
        int expected=builders.size(); // Original load consumes the builder map.
        Class.forName("adg").getMethod("a",Map.class).invoke(collection,builders);
        Class.forName("ada").getMethod("a",Class.forName("adg")).invoke(null,collection);
        Map<?,?> loaded=(Map<?,?>)Class.forName("adg").getMethod("b").invoke(collection);
        if(loaded.size()!=expected)throw new IllegalStateException("unresolved vanilla item tag: "+loaded.size()+"/"+expected);
        for(var e:loaded.entrySet()) {
            JsonArray members=new JsonArray();
            List<String> names=new ArrayList<>();
            for(Object item:(List<?>)Class.forName("adf").getMethod("b").invoke(e.getValue()))
                names.add(ExportInventoryTransfers.nameOf.invoke(ExportInventoryTransfers.items,item).toString());
            Collections.sort(names);for(String name:names)members.add(name);tags.add(e.getKey().toString(),members);
        }
    }
    static void modernContext() throws Exception {
        ExportItemComponents.loadVanillaRegistries();
        Object world=ExportInventoryTransfers.playerClass.getMethod("ao").invoke(ExportInventoryTransfers.player);
        Object server=Class.forName("axf").getMethod("s").invoke(world);
        Object recipes=Class.forName("dqz").getConstructor(Class.forName("jf$a")).newInstance(ExportItemComponents.registries);
        Object managers=ExportInventoryTransfers.allocate(Class.forName("ane"));
        ExportInventoryTransfers.field(managers,Class.forName("ane"),"e",recipes);
        Constructor<?> c=Class.forName("net.minecraft.server.MinecraftServer$a")
            .getDeclaredConstructor(Class.forName("bap"),Class.forName("ane"));c.setAccessible(true);
        ExportInventoryTransfers.field(server,Class.forName("net.minecraft.server.MinecraftServer"),"aD",c.newInstance(null,managers));
        Object flags=Class.forName("dhb").getField("h").get(null);
        Object fuels=Class.forName("emb").getMethod("a",Class.forName("jf$a"),Class.forName("dgz"))
            .invoke(null,ExportItemComponents.registries,flags);
        ExportInventoryTransfers.field(server,Class.forName("net.minecraft.server.MinecraftServer"),"aJ",fuels);
        for(Object lookup:((java.util.stream.Stream<?>)Class.forName("jf$a").getMethod("c").invoke(ExportItemComponents.registries)).toList()) {
            Object key=Class.forName("jf$b").getMethod("g").invoke(lookup);
            if(!Class.forName("amt").getMethod("a").invoke(key).toString().equals("minecraft:item"))continue;
            for(Object tag:((java.util.stream.Stream<?>)Class.forName("jf").getMethod("e").invoke(lookup)).toList()) {
                Object tagKey=Class.forName("jh$c").getMethod("h").invoke(tag);
                List<String> names=new ArrayList<>();
                for(Object holder:(Iterable<?>)tag)names.add(Class.forName("jd").getMethod("g").invoke(holder).toString());
                Collections.sort(names);JsonArray members=new JsonArray();for(String name:names)members.add(name);
                tags.add(Class.forName("bef").getMethod("b").invoke(tagKey).toString(),members);
            }
        }
    }
    static void outlines(JsonObject out,boolean old) throws Exception {
        ExportStorageOutlines.legacy=old;
        Class<?> registryClass=ExportStorageOutlines.cls("gl","jq"),blockClass=ExportStorageOutlines.cls("bvr","dzq");
        Class<?> stateClass=ExportStorageOutlines.cls("cfj","eoh"),viewClass=ExportStorageOutlines.cls("bpg","dvt");
        Class<?> positionClass=ExportStorageOutlines.cls("fu","is"),collisionClass=ExportStorageOutlines.cls("der","ftr");
        Object registry=ExportStorageOutlines.cls("gl","mi").getField(old?"aj":"e").get(null);
        Method name=registryClass.getMethod("b",Object.class);
        Map<String,Object> blocks=new TreeMap<>();
        for(Object block:(Iterable<?>)registry)blocks.put(name.invoke(registry,block).toString(),block);
        Object air=blockClass.getMethod(old?"n":"m").invoke(blocks.get("minecraft:air"));
        Object world=ExportStorageOutlines.cls("bpp","dwf").getField("a").get(null);
        Object origin=positionClass.getField(old?"b":"c").get(null),collision=collisionClass.getMethod("a").invoke(null);
        JsonArray states=new JsonArray(),rays=new JsonArray();
        for(String key:List.of("minecraft:furnace","minecraft:blast_furnace","minecraft:smoker")) {
            Object block=blocks.get(key);
            Method outline=ExportCraftingOutlines.inherited(block.getClass(),old?"b":"a",stateClass,viewClass,positionClass,collisionClass);
            Method auxiliary=ExportCraftingOutlines.inherited(block.getClass(),old?"a_":"a",stateClass,viewClass,positionClass);
            Object definition=blockClass.getMethod(old?"m":"l").invoke(block);
            for(Object state:(List<?>)ExportStorageOutlines.cls("cfk","eoi").getMethod("a").invoke(definition)) {
                JsonObject row=new JsonObject(),nativeState=new JsonObject(),properties=new JsonObject();nativeState.addProperty("name",key);
                for(var entry:((Map<?,?>)ExportStorageOutlines.cls("cfl","eoj").getMethod(old?"s":"G").invoke(state)).entrySet()) {
                    Class<?> property=ExportStorageOutlines.cls("cgl","epk");
                    properties.addProperty((String)property.getMethod("f").invoke(entry.getKey()),(String)property.getMethod(old?"a":"b",Comparable.class).invoke(entry.getKey(),entry.getValue()));
                }
                nativeState.add("properties",properties);row.add("state",nativeState);
                row.addProperty("native_id",(Integer)blockClass.getMethod(old?"i":"j",stateClass).invoke(null,state));
                row.add("outline",ExportStorageOutlines.boxes(outline.invoke(block,state,world,origin,collision)));
                row.add("auxiliary",ExportStorageOutlines.boxes(auxiliary.invoke(block,state,world,origin)));
                int index=states.size();states.add(row);
                for(int axis=0;axis<3;axis++) {
                    double[] a={.5,.5,.5},b=a.clone();a[axis]=-2;b[axis]=2;
                    rays.add(ExportStorageOutlines.sample(state,air,index,0,a,b));rays.add(ExportStorageOutlines.sample(state,air,index,0,b,a));
                }
            }
        }
        if(states.size()!=24||rays.size()!=144)throw new IllegalStateException("native furnace state coverage changed");
        out.add("states",states);out.add("rays",rays);
    }
    public static void main(String[] args) throws Exception {
        ExportInventoryTransfers.init(args[0]);boolean old=ExportInventoryTransfers.legacy;
        if(old)loadLegacyTags(args[2]);else modernContext();
        JsonObject out=new JsonObject();out.addProperty("version",args[0]);out.add("vanilla_item_tags",tags);
        JsonObject dependencies=new JsonObject();
        for(String field:args[5].split(",")) {
            Object tag=Class.forName(old?"ada":"bdy").getField(field).get(null);
            String name=Class.forName(old?"adf$e":"bef").getMethod(old?"a":"b").invoke(tag).toString();
            if(!tags.has(name))throw new IllegalStateException("native fuel tag missing: "+name);
            dependencies.add(name,tags.get(name));
        }
        out.add("fuel_dependency_tags",dependencies);
        JsonArray menus=new JsonArray();
        for(String name:List.of("minecraft:furnace","minecraft:blast_furnace","minecraft:smoker")) {
            ExportInventoryTransfers.reset();
            String cls=name.equals("minecraft:furnace")?(old?"bha":"dih"):args[name.equals("minecraft:blast_furnace")?3:4];
            Object menu=Class.forName(cls).getConstructor(int.class,ExportInventoryTransfers.inventoryClass).newInstance(3,ExportInventoryTransfers.inventory);
            List<?> slots=ExportInventoryTransfers.slots(menu);JsonObject row=new JsonObject();row.addProperty("name",name);row.addProperty("total_slots",slots.size());row.addProperty("native_class",cls);
            for(Object type:(Iterable<?>)ExportInventoryTransfers.menus)
                if(ExportInventoryTransfers.nameOf.invoke(ExportInventoryTransfers.menus,type).toString().equals(name))
                    row.addProperty("native_id",(Integer)ExportInventoryTransfers.idOf.invoke(ExportInventoryTransfers.menus,type));
            JsonArray players=new JsonArray(),policies=new JsonArray();
            Field container=ExportInventoryTransfers.slotClass.getField("c"),rawIndex=ExportInventoryTransfers.slotClass.getDeclaredField("a");rawIndex.setAccessible(true);
            for(int i=0;i<slots.size();i++) {
                Object slot=slots.get(i);JsonObject p=new JsonObject();p.addProperty("slot",i);p.addProperty("native_class",slot.getClass().getName());
                p.addProperty("may_pickup",(Boolean)ExportInventoryTransfers.mayPickup.invoke(slot,ExportInventoryTransfers.player));
                int capacity=(Integer)ExportInventoryTransfers.slotClass.getMethod("a").invoke(slot);p.addProperty("base_capacity",capacity);
                JsonArray refused=new JsonArray();JsonObject overrides=new JsonObject();
                for(String item:ExportInventoryTransfers.byName.keySet()) {
                    Object stack=ExportInventoryTransfers.stack(item,1);
                    if(!(Boolean)ExportInventoryTransfers.mayPlace.invoke(slot,stack))refused.add(item);
                    int effective=(Integer)ExportInventoryTransfers.slotClass.getMethod(old?"b":"b_",ExportInventoryTransfers.stackClass).invoke(slot,stack);
                    int nativeItemCapacity=(Integer)ExportInventoryTransfers.stackClass.getMethod(old?"c":"k").invoke(stack);
                    if(effective!=Math.min(capacity,nativeItemCapacity))overrides.addProperty(item,effective);
                }
                p.add("rejected_default_items",refused);p.add("capacity_overrides",overrides);policies.add(p);
                if(container.get(slot)==ExportInventoryTransfers.inventory) {
                    JsonObject mapping=new JsonObject();mapping.addProperty("screen_slot",i);mapping.addProperty("raw_player_slot",rawIndex.getInt(slot));players.add(mapping);
                } else p.addProperty("container_index",rawIndex.getInt(slot));
            }
            if(slots.size()!=39||players.size()!=36)throw new IllegalStateException("native furnace topology changed");
            row.add("player_slots",players);row.add("slot_policies",policies);menus.add(row);
        }
        out.add("menus",menus);
        outlines(out,old);
        Files.writeString(Path.of(args[1]),new GsonBuilder().setPrettyPrinting().create().toJson(out)+"\n");
        if(old)Class.forName("v").getMethod("h").invoke(null);else Class.forName("bas").getMethod("close").invoke(ExportItemComponents.resourceManager);
    }
}
