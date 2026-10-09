// Original crafting constructors/slot methods, without recipe execution or prediction.
import com.google.gson.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
public final class ExportCraftingMenus {
    public static void main(String[] args) throws Exception {
        String version=args[0];
        ExportInventoryTransfers.init(version);
        boolean legacy=ExportInventoryTransfers.legacy;
        JsonArray menus=new JsonArray();
        for (String name:List.of("minecraft:player","minecraft:crafting")) {
            ExportInventoryTransfers.reset();
            Object menu=name.equals("minecraft:player")?ExportInventoryTransfers.menu(name):
                Class.forName(legacy?"bgv":"dic").getConstructor(int.class,ExportInventoryTransfers.inventoryClass).newInstance(3,ExportInventoryTransfers.inventory);
            List<?> slots=ExportInventoryTransfers.slots(menu);
            Class<?> gridClass=Class.forName(legacy?"bgu":"dib");
            Class<?> resultClass=Class.forName(legacy?"bhs":"dje");
            Field containerField=ExportInventoryTransfers.slotClass.getField("c");
            Field rawIndex=ExportInventoryTransfers.slotClass.getDeclaredField("a");rawIndex.setAccessible(true);
            JsonObject row=new JsonObject();row.addProperty("name",name);row.addProperty("total_slots",slots.size());
            if (name.equals("minecraft:crafting")) {
                for (Object type:(Iterable<?>)ExportInventoryTransfers.menus)
                    if (ExportInventoryTransfers.nameOf.invoke(ExportInventoryTransfers.menus,type).toString().equals(name))
                        row.addProperty("native_id",(Integer)ExportInventoryTransfers.idOf.invoke(ExportInventoryTransfers.menus,type));
            }
            JsonArray inputs=new JsonArray(),playerSlots=new JsonArray(),policies=new JsonArray();
            Object grid=null;
            for (int i=0;i<slots.size();i++) {
                Object slot=slots.get(i),container=containerField.get(slot);
                boolean input=gridClass.isInstance(container),result=resultClass.isInstance(slot),player=container==ExportInventoryTransfers.inventory;
                if (input) { grid=container;JsonObject cell=new JsonObject();cell.addProperty("screen_slot",i);cell.addProperty("grid_index",rawIndex.getInt(slot));inputs.add(cell); }
                if (result) row.addProperty("result_slot",i);
                if (player) {JsonObject cell=new JsonObject();cell.addProperty("screen_slot",i);cell.addProperty("raw_player_slot",rawIndex.getInt(slot));playerSlots.add(cell);}
                // Existing armor/offhand rules remain separate; inspect grid and
                // the crafting table's ordinary appended player slots only.
                if (!(input||result||(name.equals("minecraft:crafting")&&player))) continue;
                JsonObject policy=new JsonObject();policy.addProperty("slot",i);policy.addProperty("native_class",slot.getClass().getName());
                policy.addProperty("may_pickup_empty",(Boolean)ExportInventoryTransfers.mayPickup.invoke(slot,ExportInventoryTransfers.player));
                policy.addProperty("base_capacity",(Integer)ExportInventoryTransfers.slotClass.getMethod("a").invoke(slot));
                JsonArray rejected=new JsonArray();
                for (String item:ExportInventoryTransfers.byName.keySet())
                    if (!(Boolean)ExportInventoryTransfers.mayPlace.invoke(slot,ExportInventoryTransfers.stack(item,1))) rejected.add(item);
                policy.add("rejected_default_items",rejected);policies.add(policy);
            }
            if(grid==null||!row.has("result_slot"))throw new IllegalStateException("native crafting roles absent");
            row.addProperty("grid_width",(Integer)gridClass.getMethod(legacy?"g":"aB_").invoke(grid));
            row.addProperty("grid_height",(Integer)gridClass.getMethod(legacy?"f":"h").invoke(grid));
            row.add("input_slots",inputs);row.add("player_slots",playerSlots);row.add("slot_policies",policies);menus.add(row);
        }
        JsonObject out=new JsonObject();out.addProperty("version",version);out.add("menus",menus);
        Files.writeString(Path.of(args[1]),new GsonBuilder().setPrettyPrinting().create().toJson(out)+"\n");
        if(legacy)Class.forName("v").getMethod("h").invoke(null);
    }
}
