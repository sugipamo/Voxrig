// Own caller of complete original crafting matchers; no replacement game code.
// Genuine CraftingContainer/CraftingInput constructors hold actual original stacks.
// The unused menu/Level arguments are null; no menu callback or world path is called.
import com.google.gson.*;
import com.mojang.serialization.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
public final class ExportRecipeGridMatching {
    @SuppressWarnings("unchecked")
    public static void main(String[] args) throws Exception {
        ExportItemProperties.init(args[0]);boolean old=ExportItemProperties.legacy;
        ExportCraftingReturns.old=old;
        if(!old) {
            for(Object type:(Iterable<?>)ExportItemProperties.components)ExportCraftingReturns.types.put(ExportItemProperties.name.invoke(ExportItemProperties.components,type).toString(),type);
            ExportCraftingReturns.ops=(DynamicOps<JsonElement>)Class.forName("ams").getMethod("a",DynamicOps.class,Class.forName("jf$a")).invoke(null,JsonOps.INSTANCE,ExportItemComponents.registries);
        }
        JsonObject out=new JsonObject();out.addProperty("version",args[0]);out.addProperty("java_version",System.getProperty("java.version"));JsonArray cases=new JsonArray();
        for(JsonElement request:new JsonParser().parse(Files.readString(Path.of(args[1]))).getAsJsonArray()) {
            JsonObject row=new JsonParser().parse(request.toString()).getAsJsonObject();
            int width=row.get("grid_width").getAsInt(),height=row.get("grid_height").getAsInt();
            List<Object> stacks=new ArrayList<>();JsonArray encoded=new JsonArray();
            for(JsonElement input:row.getAsJsonArray("inputs")) {
                Object value=ExportCraftingReturns.stack(input);stacks.add(value);encoded.add(ExportItemProperties.encoded(value));
            }
            if(stacks.size()!=width*height)throw new IllegalStateException("grid count");
            Object grid;
            if(old) {
                Class<?> container=Class.forName("bgu");grid=container.getConstructor(Class.forName("bgi"),int.class,int.class).newInstance(null,width,height);
                Field items=container.getDeclaredField("a");items.setAccessible(true);
                List<Object> contents=(List<Object>)items.get(grid);for(int i=0;i<stacks.size();i++)contents.set(i,stacks.get(i));
            } else grid=Class.forName("dqg").getMethod("a",int.class,int.class,List.class).invoke(null,width,height,stacks);
            Class<?> ingredient=Class.forName(old?"bmr":"dqo");List<Object> ingredients=new ArrayList<>();
            for(JsonElement choices:row.getAsJsonArray("ingredients")) {
                if(choices.isJsonNull()) {ingredients.add(old?ingredient.getField("a").get(null):Optional.empty());continue;}
                Object value;
                if(old) {
                    JsonArray names=new JsonArray();for(JsonElement name:choices.getAsJsonArray()){JsonObject item=new JsonObject();item.addProperty("item",name.getAsString());names.add(item);}
                    value=ingredient.getMethod("a",JsonElement.class).invoke(null,names);
                } else value=((Codec<Object>)ingredient.getField("d").get(null)).parse(ExportCraftingReturns.ops,choices).getOrThrow();
                ingredients.add(old?value:Optional.of(value));
            }
            boolean shaped=row.get("shaped").getAsBoolean();Object result=ExportItemProperties.defaultStack(ExportItemProperties.byName.get("minecraft:stone"));Object recipe;
            if(old) {
                Object list=Class.forName("gi").getMethod("a",int.class,Object.class).invoke(null,ingredients.size(),ingredient.getField("a").get(null));
                List<Object> entries=(List<Object>)list;for(int i=0;i<ingredients.size();i++)entries.set(i,ingredients.get(i));
                Class<?> identity=Class.forName("uh");Object id=identity.getConstructor(String.class).newInstance("voxrig:grid_match");
                if(shaped)recipe=Class.forName("bmz").getConstructor(identity,String.class,int.class,int.class,Class.forName("gi"),ExportItemProperties.stack).newInstance(id,"",row.get("recipe_width").getAsInt(),row.get("recipe_height").getAsInt(),list,result);
                else recipe=Class.forName("bna").getConstructor(identity,String.class,ExportItemProperties.stack,Class.forName("gi")).newInstance(id,"",result,list);
            } else {
                Class<?> category=Class.forName("dqf");Object misc=Arrays.stream(category.getEnumConstants()).filter(v->((Enum<?>)v).name().equals("MISC")).findFirst().orElseThrow();
                if(shaped) {
                    Object pattern=Class.forName("drh").getConstructor(int.class,int.class,List.class,Optional.class).newInstance(row.get("recipe_width").getAsInt(),row.get("recipe_height").getAsInt(),ingredients,Optional.empty());
                    recipe=Class.forName("drg").getConstructor(String.class,category,Class.forName("drh"),ExportItemProperties.stack).newInstance("",misc,pattern,result);
                } else {
                    List<Object> nonempty=new ArrayList<>();for(Object value:ingredients)nonempty.add(((Optional<?>)value).orElseThrow());
                    recipe=Class.forName("dri").getConstructor(String.class,category,ExportItemProperties.stack,List.class).newInstance("",misc,result,nonempty);
                }
                row.addProperty("normalized_width",(Integer)Class.forName("dqg").getMethod("f").invoke(grid));
                row.addProperty("normalized_height",(Integer)Class.forName("dqg").getMethod("g").invoke(grid));
            }
            boolean matches=(Boolean)recipe.getClass().getMethod("a",Class.forName(old?"bgu":"dqg"),Class.forName(old?"bqb":"dwo")).invoke(recipe,grid,null);
            row.add("inputs_encoded",encoded);row.addProperty("matches",matches);cases.add(row);
        }
        out.add("cases",cases);Files.writeString(Path.of(args[2]),new GsonBuilder().setPrettyPrinting().create().toJson(out)+"\n");
        if(!old)Class.forName("bas").getMethod("close").invoke(ExportItemComponents.resourceManager);
        System.out.println(args[0]+" original entire grid matchers="+cases.size());
    }
}
