// Original tooling for the unmodified official 1.16.1 JAR. No game code is copied.
// The generic ordinary slot isolates PICKUP's returned predecessor and packet
// comparison semantics. Furnace processing itself is qualified by live tests.
import com.google.gson.*;
import java.nio.file.*;
public final class VerifyLegacyFurnaceComparison {
    public static void main(String[] args) throws Exception {
        ExportRegularClicks.init("1.16.1");
        JsonArray rows = new JsonArray();
        for (int button=0;button<=1;button++) for(int count=0;count<=64;count++) {
            Object menu=ExportRegularClicks.menu("minecraft:generic_9x1");
            Object slot=ExportRegularClicks.slots(menu).get(0);
            ExportRegularClicks.slotSet(slot,ExportRegularClicks.stack("minecraft:coal",count));
            ExportRegularClicks.cursor(menu,ExportRegularClicks.stack("minecraft:coal",3));
            Object result=ExportRegularClicks.menuClass.getMethod("a",int.class,int.class,
                Class.forName("bgq"),ExportRegularClicks.playerClass).invoke(menu,0,button,
                    ExportRegularClicks.pickup,ExportRegularClicks.player);
            Object marker=ExportRegularClicks.stack("minecraft:coal",127);
            if ((boolean)ExportRegularClicks.stackClass.getMethod("b",ExportRegularClicks.stackClass,
                    ExportRegularClicks.stackClass).invoke(null,marker,result))
                throw new IllegalStateException("resync marker matched ordinary predecessor");
            byte[] packet=ExportRegularClicks.legacyPacket(marker,1,1,0,button);
            JsonObject row=new JsonObject();row.addProperty("button",button);row.addProperty("actual_before",count);
            row.add("native_return",ExportRegularClicks.describe(result));
            row.add("slot_after",ExportRegularClicks.describe(ExportRegularClicks.slotGet(slot)));
            row.add("cursor_after",ExportRegularClicks.describe(ExportRegularClicks.cursor(menu)));
            row.addProperty("marker_count",127);row.addProperty("marker_matched",false);
            row.addProperty("native_codec_roundtrip",true);
            row.addProperty("payload_hex",java.util.HexFormat.of().formatHex(packet));rows.add(row);
        }
        Files.writeString(Path.of(args[0]),new GsonBuilder().setPrettyPrinting().create().toJson(rows)+"\n");
        System.out.println("130 native PICKUP predecessors and comparison-only marker roundtrips verified");
        Class.forName("v").getMethod("h").invoke(null);
    }
}
