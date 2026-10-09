// Own wrapper of original recipe ingredient constructors and stock picker.
// No player/world stubs or replacement game implementations are used.
import com.google.gson.*;
import com.mojang.serialization.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
public final class ExportRecipeBookMatching {
    @SuppressWarnings("unchecked")
    public static void main(String[] args) throws Exception {
        String version=args[0];ExportItemProperties.init(version);boolean old=ExportItemProperties.legacy;
        DynamicOps<JsonElement> ops=null;
        if(!old)ops=(DynamicOps<JsonElement>)Class.forName("ams").getMethod("a",DynamicOps.class,Class.forName("jf$a")).invoke(null,JsonOps.INSTANCE,ExportItemComponents.registries);
        JsonObject out=new JsonObject();out.addProperty("version",version);out.addProperty("java_version",System.getProperty("java.version"));JsonArray cases=new JsonArray();
        for(JsonElement input:new JsonParser().parse(Files.readString(Path.of(args[1]))).getAsJsonArray()) {
            JsonObject row=new JsonParser().parse(input.toString()).getAsJsonObject();Object stocks=Class.forName(old?"bee":"ddu").getConstructor().newInstance();
            for(JsonElement stock:row.getAsJsonArray("stocks")) {
                JsonObject request=stock.getAsJsonObject();Object item=Objects.requireNonNull(ExportItemProperties.byName.get(request.get("item").getAsString()));
                Object stack=ExportItemProperties.defaultStack(item);ExportItemProperties.stack.getMethod("e",int.class).invoke(stack,request.get("count").getAsInt());
                stocks.getClass().getMethod("a",ExportItemProperties.stack).invoke(stocks,stack);
            }
            int crafts=row.get("crafts").getAsInt(),bound=row.get("bound").getAsInt();boolean possible;int maximum;
            if(old) {
                JsonObject recipeJson=new JsonObject();recipeJson.addProperty("group","");JsonArray ingredients=new JsonArray();
                for(JsonElement choices:row.getAsJsonArray("ingredients")) {
                    JsonArray alternatives=new JsonArray();for(JsonElement name:choices.getAsJsonArray()){JsonObject item=new JsonObject();item.addProperty("item",name.getAsString());alternatives.add(item);}ingredients.add(alternatives);
                }
                recipeJson.add("ingredients",ingredients);JsonObject result=new JsonObject();result.addProperty("item","minecraft:stone");recipeJson.add("result",result);
                Object serializer=Class.forName("bna$a").getConstructor().newInstance();Class<?> identity=Class.forName("uh");Object recipe=serializer.getClass().getMethod("b",identity,JsonObject.class).invoke(serializer,identity.getConstructor(String.class).newInstance("voxrig:matching"),recipeJson);
                Class<?> recipeType=Class.forName("bmu"),output=Class.forName("it.unimi.dsi.fastutil.ints.IntList");
                possible=(boolean)stocks.getClass().getMethod("a",recipeType,output,int.class).invoke(stocks,recipe,null,crafts);
                maximum=(int)stocks.getClass().getMethod("a",recipeType,int.class,output).invoke(stocks,recipe,bound,null);
            } else {
                List<Object> ingredients=new ArrayList<>();Codec<Object> ingredient=(Codec<Object>)Class.forName("dqo").getField("d").get(null);
                for(JsonElement choices:row.getAsJsonArray("ingredients"))ingredients.add(ingredient.parse(ops,choices).getOrThrow());
                Class<?> output=Class.forName("ddt$b");Method pick=stocks.getClass().getDeclaredMethod("a",List.class,int.class,output);pick.setAccessible(true);
                possible=(boolean)pick.invoke(stocks,ingredients,crafts,null);
                Field raw=stocks.getClass().getDeclaredField("a");raw.setAccessible(true);Object contents=raw.get(stocks);
                maximum=(int)contents.getClass().getMethod("b",List.class,int.class,output).invoke(contents,ingredients,bound,null);
            }
            row.addProperty("possible",possible);row.addProperty("maximum",maximum);cases.add(row);
        }
        out.add("cases",cases);Files.writeString(Path.of(args[2]),new GsonBuilder().setPrettyPrinting().create().toJson(out)+"\n");
        if(!old)Class.forName("bas").getMethod("close").invoke(ExportItemComponents.resourceManager);
        System.out.println(version+" native recipe book matching cases="+cases.size());
    }
}
