// Own observer of native default-stack mining getters for every block state; no game body is replaced.
import com.google.gson.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
import java.util.function.Function;
import java.util.zip.ZipFile;

public final class ExportDigProfiles {
    static Class<?> cls(String oldName,String newName) throws Exception {
        return ExportStorageOutlines.cls(oldName,newName);
    }
    static JsonObject legacyTags(Path jar,Object blocks) throws Exception {
        Class<?> collection=Class.forName("adg"),builder=Class.forName("adf$a"),location=Class.forName("uh");
        Map<Object,Object> builders=new LinkedHashMap<>();
        try(ZipFile zip=new ZipFile(jar.toFile())) {
            for(var entry:Collections.list(zip.entries())) {
                String path=entry.getName(),prefix="data/minecraft/tags/blocks/";
                if(!path.startsWith(prefix)||!path.endsWith(".json"))continue;
                Object key=location.getConstructor(String.class).newInstance("minecraft:"+path.substring(prefix.length(),path.length()-5));
                Object value=builder.getMethod("a").invoke(null);
                String json=new String(zip.getInputStream(entry).readAllBytes(),java.nio.charset.StandardCharsets.UTF_8);
                builder.getMethod("a",JsonObject.class,String.class).invoke(value,new JsonParser().parse(json).getAsJsonObject(),"original vanilla JAR");
                builders.put(key,value);
            }
        }
        Method lookup=Class.forName("gl").getMethod("a",location);
        Function<Object,Optional<Object>> resolver=key->{try{return Optional.of(lookup.invoke(blocks,key));}catch(Exception e){throw new IllegalStateException(e);}};
        Object loaded=collection.getConstructor(Function.class,String.class,String.class).newInstance(resolver,"tags/blocks","blocks");
        collection.getMethod("a",Map.class).invoke(loaded,builders);
        Class.forName("acx").getMethod("a",collection).invoke(null,loaded);
        if(!((Set<?>)Class.forName("acx").getMethod("b",collection).invoke(null,loaded)).isEmpty())throw new IllegalStateException("missing vanilla block tags");
        JsonObject out=new JsonObject();
        Map<?,?> values=(Map<?,?>)collection.getMethod("b").invoke(loaded);
        Method id=Class.forName("gl").getMethod("a",Object.class);
        for(var entry:values.entrySet()) {
            TreeSet<Integer> ids=new TreeSet<>();
            for(Object block:(List<?>)Class.forName("adf").getMethod("b").invoke(entry.getValue()))ids.add((Integer)id.invoke(blocks,block));
            JsonArray row=new JsonArray();ids.forEach(row::add);out.add(entry.getKey().toString(),row);
        }
        return out;
    }
    static JsonObject modernTags(Object blocks) throws Exception {
        JsonObject out=new JsonObject();
        Method id=Class.forName("jq").getMethod("a",Object.class);
        try(var stream=(java.util.stream.Stream<?>)Class.forName("jq").getMethod("l").invoke(blocks)) {
            for(Object tag:stream.toList()) {
                Object key=Class.forName("jh$c").getMethod("h").invoke(tag);
                String name=Class.forName("bef").getMethod("b").invoke(key).toString();
                TreeSet<Integer> ids=new TreeSet<>();
                for(Object holder:(Iterable<?>)tag)ids.add((Integer)id.invoke(blocks,Class.forName("jd").getMethod("a").invoke(holder)));
                JsonArray row=new JsonArray();ids.forEach(row::add);out.add(name,row);
            }
        }
        return out;
    }
    public static void main(String[] args) throws Exception {
        ExportItemProperties.init(args[0]);boolean old=ExportItemProperties.legacy;
        ExportStorageOutlines.legacy=old;
        try {
            Object blocks=cls("gl","mi").getField(old?"aj":"e").get(null);
            JsonObject tags=old?legacyTags(Path.of(args[1]),blocks):modernTags(blocks);
            Method key=cls("gl","jq").getMethod("b",Object.class),blockId=cls("gl","jq").getMethod("a",Object.class);
            Class<?> blockClass=cls("bvr","dzq"),stateBase=cls("cfi$a","eog$a"),stateClass=cls("cfj","eoh");
            Object world=cls("bpp","dwf").getField("a").get(null),origin=cls("fu","is").getField(old?"b":"c").get(null);
            Method hardness=stateBase.getMethod(old?"h":"e",cls("bpg","dvt"),cls("fu","is")),needs=stateBase.getMethod(old?"q":"C");
            Method speed=ExportItemProperties.stack.getMethod("a",stateClass),correct=ExportItemProperties.stack.getMethod("b",stateClass);
            Map<String,Object> stacks=new TreeMap<>();JsonArray items=new JsonArray();
            for(var e:ExportItemProperties.byName.entrySet()) {
                Object stack=ExportItemProperties.defaultStack(e.getValue());
                if((Boolean)ExportItemProperties.stack.getMethod(old?"a":"f").invoke(stack))continue;
                stacks.put(e.getKey(),stack);JsonObject row=new JsonObject();row.addProperty("name",e.getKey());row.addProperty("native_id",(Integer)ExportItemProperties.id.invoke(ExportItemProperties.items,e.getValue()));items.add(row);
            }
            Map<String,Object> registered=new TreeMap<>();for(Object block:(Iterable<?>)blocks)registered.put(key.invoke(blocks,block).toString(),block);
            JsonArray profiles=new JsonArray(),states=new JsonArray();Map<String,Integer> profileIds=new LinkedHashMap<>();long comparisons=0;
            for(var entry:registered.entrySet()) {
                Object block=entry.getValue();
                Object definition=blockClass.getMethod(old?"m":"l").invoke(block);
                for(Object state:(List<?>)cls("cfk","eoi").getMethod("a").invoke(definition)) {
                    Map<String,String> properties=new TreeMap<>();Map<?,?> nativeProperties=(Map<?,?>)cls("cfl","eoj").getMethod(old?"s":"G").invoke(state);
                    for(var e:nativeProperties.entrySet())properties.put((String)cls("cgl","epk").getMethod("f").invoke(e.getKey()),(String)cls("cgl","epk").getMethod(old?"a":"b",Comparable.class).invoke(e.getKey(),e.getValue()));
                    float h=(Float)hardness.invoke(state,world,origin);boolean need=(Boolean)needs.invoke(state);
                    if(!Float.isFinite(h))throw new IllegalStateException("non-finite hardness");
                    JsonObject profile=new JsonObject();profile.addProperty("hardness",h);profile.addProperty("hand_harvestable",!need);JsonArray overrides=new JsonArray();
                    for(var e:stacks.entrySet()) {
                        float s=(Float)speed.invoke(e.getValue(),state);boolean harvest=!need||(Boolean)correct.invoke(e.getValue(),state);comparisons++;
                        if(!Float.isFinite(s)||s<=0)throw new IllegalStateException("invalid native speed");
                        if(s!=1f||harvest!=!need) {
                            JsonObject tool=new JsonObject();tool.addProperty("item",e.getKey());tool.addProperty("speed",s);tool.addProperty("harvestable",harvest);overrides.add(tool);
                        }
                    }
                    profile.add("item_overrides",overrides);String signature=profile.toString();Integer index=profileIds.get(signature);
                    if(index==null){index=profiles.size();profiles.add(profile);profileIds.put(signature,index);}
                    JsonArray row=new JsonArray();row.add((Integer)blockClass.getMethod(old?"i":"j",stateClass).invoke(null,state));row.add((Integer)blockId.invoke(blocks,block));row.add(index);states.add(row);
                }
            }
            JsonObject out=new JsonObject();out.addProperty("version",args[0]);out.add("block_tags",tags);out.add("items",items);out.add("profiles",profiles);out.add("states",states);out.addProperty("native_item_state_comparisons",comparisons);
            Files.writeString(Path.of(args[2]),new Gson().toJson(out)+"\n");
            System.out.println(args[0]+": "+items.size()+" items, "+states.size()+" states, "+profiles.size()+" mining profiles, "+comparisons+" native item/state getters");
        } finally {
            if(old)Class.forName("v").getMethod("h").invoke(null);
            else ((AutoCloseable)ExportItemComponents.resourceManager).close();
        }
    }
}
