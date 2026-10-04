import com.google.gson.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.util.*;
public class ExportItemComponentSchema {
 static Class<?> codec;static Object access;static IdentityHashMap<Object,Integer> ids=new IdentityHashMap<>();static JsonArray nodes=new JsonArray();
 static Object field(Object value,String name)throws Exception {Field f=value.getClass().getDeclaredField(name);f.setAccessible(true);return f.get(value);}
 static Object registry(Object selector)throws Exception {return Class.forName("jr").getMethod("f",Class.forName("amt")).invoke(access,field(selector,"b"));}
 @SuppressWarnings("unchecked") static java.util.function.Function<Object,Object> function(Object value,String name)throws Exception {return (java.util.function.Function<Object,Object>)field(value,name);}
 @SuppressWarnings("unchecked") static int node(Object value)throws Exception {
  if(ids.containsKey(value))return ids.get(value);
  int id=ids.size();ids.put(value,id);JsonObject n=new JsonObject();n.addProperty("id",id);n.addProperty("class",value.getClass().getName());nodes.add(n);JsonArray fields=new JsonArray();n.add("fields",fields);
  for(Field f:value.getClass().getDeclaredFields())if(!Modifier.isStatic(f.getModifiers())) {
   f.setAccessible(true);Object v=f.get(value);JsonObject field=new JsonObject();field.addProperty("name",f.getName());field.addProperty("type",f.getType().getName());
   if(v!=null) {
    field.addProperty("value_class",v.getClass().getName());
    if(codec.isInstance(v))field.addProperty("codec",node(v));
    else if(v instanceof Number||v instanceof Boolean||v instanceof String)field.addProperty("value",v.toString());
    else if(v.getClass().getName().equals("amt"))field.addProperty("key",v.toString());
   }
   fields.add(field);
  }
  if(value.getClass().getName().equals("aao$11"))n.addProperty("resolved",node(((java.util.function.Supplier<?>)field(value,"b")).get()));
  if(value.getClass().getName().equals("aao$16")) {
   Object selector=field(value,"c");JsonArray variants=new JsonArray();n.add("variants",variants);var choose=function(value,"a");
   if(selector.getClass().getName().equals("aam$21")) {
    Object first=((java.util.function.IntFunction<?>)field(selector,"a")).apply(0);
    for(Object e:first.getClass().getEnumConstants()) {JsonObject v=new JsonObject();v.addProperty("tag",((java.util.function.ToIntFunction<Object>)field(selector,"b")).applyAsInt(e));v.addProperty("codec",node(choose.apply(e)));variants.add(v);}
   } else if(selector.getClass().getName().equals("aam$22")) {
    Object reg=registry(selector);
    for(Object e:(Iterable<?>)reg) {JsonObject v=new JsonObject();v.addProperty("tag",(int)ExportInventoryTransfers.idOf.invoke(reg,e));v.addProperty("name",ExportInventoryTransfers.nameOf.invoke(reg,e).toString());v.addProperty("codec",node(choose.apply(e)));variants.add(v);}
   } else {
    Object either=field(selector,"c");
    for(String side:new String[]{"a","b"}) {Object reg=registry(field(either,side));
     for(Object e:(Iterable<?>)reg) {Object wrapped=com.mojang.datafixers.util.Either.class.getMethod(side.equals("a")?"left":"right",Object.class).invoke(null,e);JsonObject v=new JsonObject();v.addProperty("left",side.equals("a"));v.addProperty("tag",(int)ExportInventoryTransfers.idOf.invoke(reg,e));v.addProperty("name",ExportInventoryTransfers.nameOf.invoke(reg,e).toString());v.addProperty("codec",node(choose.apply(function(selector,"a").apply(wrapped))));variants.add(v);}
    }
   }
  }
  return id;
 }
 public static void main(String[] args)throws Exception {
  ExportInventoryTransfers.init("1.21.11");codec=Class.forName("aao");Object registry=Class.forName("mi").getField("am").get(null);access=Class.forName("jr").getMethod("a",ExportInventoryTransfers.registryClass).invoke(null,Class.forName("mi").getField("aR").get(null));JsonObject roots=new JsonObject();
  for(Object type:(Iterable<?>)registry)roots.addProperty(ExportInventoryTransfers.nameOf.invoke(registry,type).toString(),node(Class.forName("kh").getMethod("f").invoke(type)));
  roots.addProperty("native_item",node(ExportInventoryTransfers.stackClass.getField("h").get(null)));
  JsonObject out=new JsonObject();out.add("roots",roots);out.add("nodes",nodes);Files.writeString(Path.of(args[0]),new GsonBuilder().setPrettyPrinting().create().toJson(out)+"\n");
 }
}
