// Own observer of unchanged official team/player-info codecs and native membership rules.
import com.google.gson.*;
import io.netty.buffer.*;
import java.lang.reflect.*;
import java.nio.file.*;
import java.security.*;
import java.time.Instant;
import java.util.*;
public final class ExportSocialPackets {
    static boolean old;static Object registries;static byte[] publicKey;
    static Object field(Object o,String name)throws Exception{Field f=o.getClass().getDeclaredField(name);f.setAccessible(true);return f.get(o);}
    static Object call(Object o,String name)throws Exception{return o.getClass().getMethod(name).invoke(o);}
    static Object buffer(byte[] raw)throws Exception{ByteBuf b=raw==null?Unpooled.buffer():Unpooled.wrappedBuffer(raw);return old?Class.forName("mg").getConstructor(ByteBuf.class).newInstance(b):Class.forName("xq").getConstructor(ByteBuf.class,Class.forName("jr")).newInstance(b,registries);}
    static byte[] bytes(Object o){ByteBuf b=(ByteBuf)o;byte[] raw=new byte[b.readableBytes()];b.getBytes(b.readerIndex(),raw);return raw;}
    static void integer(ByteBuf b,int n){do{int next=n&127;n>>>=7;b.writeByte(next|(n!=0?128:0));}while(n!=0);}
    static void string(ByteBuf b,String s){byte[] raw=s.getBytes(java.nio.charset.StandardCharsets.UTF_8);integer(b,raw.length);b.writeBytes(raw);}
    static void text(ByteBuf b,String s){if(old)string(b,"{\"text\":\""+s+"\"}");else{byte[] raw=s.getBytes(java.nio.charset.StandardCharsets.UTF_8);b.writeByte(8);b.writeShort(raw.length);b.writeBytes(raw);}}
    static void uuid(ByteBuf b,int offset){for(int i=1;i<=16;i++)b.writeByte(i+offset);}
    static JsonArray uuid(Object o){UUID u=(UUID)o;ByteBuf b=Unpooled.buffer();try{b.writeLong(u.getMostSignificantBits());b.writeLong(u.getLeastSignificantBits());return array(bytes(b));}finally{b.release();}}
    static JsonArray array(byte[] bytes){JsonArray a=new JsonArray();for(byte value:bytes)a.add(value&255);return a;}
    static JsonElement textFact(Object value)throws Exception{return value==null?JsonNull.INSTANCE:new JsonPrimitive((String)Class.forName(old?"mr":"yh").getMethod("getString").invoke(value));}
    static JsonObject readWrite(String group,int id,byte[] raw)throws Exception{
        String cls=group.equals("team")?(old?"qh":"agz"):(old?"pi":id==0x43?"afm":"afn");Class<?> type=Class.forName(cls);Object read=buffer(raw),write=buffer(null),packet;JsonObject row=new JsonObject();
        try{
            if(old){packet=type.getConstructor().newInstance();type.getMethod("a",Class.forName("mg")).invoke(packet,read);type.getMethod("b",Class.forName("mg")).invoke(packet,write);}
            else{Object codec=type.getField("a").get(null);Class<?> stream=Class.forName("aao");packet=stream.getMethod("decode",Object.class).invoke(codec,read);stream.getMethod("encode",Object.class,Object.class).invoke(codec,write,packet);}
            if(((ByteBuf)read).isReadable())throw new IllegalStateException("original social reader left fields");
            row.addProperty("group",group);row.addProperty("packet_id",id);row.addProperty("payload_hex",HexFormat.of().formatHex(bytes(write)));
            if(group.equals("team")){
                int op=(int)field(packet,old?"i":"i");row.addProperty("operation",op);row.addProperty("name",(String)field(packet,old?"a":"j"));
                if(op==0||op==2){Object p=old?packet:((Optional<?>)field(packet,"l")).orElseThrow();JsonObject params=new JsonObject();params.add("display",textFact(field(p,old?"b":"a")));params.addProperty("friendly_flags",((int)field(p,old?"j":"g"))&255);params.addProperty("visibility",old?(String)field(p,"e"):((Enum<?>)field(p,"d")).name());params.addProperty("collision",old?(String)field(p,"f"):((Enum<?>)field(p,"e")).name());params.addProperty("color",((Enum<?>)field(p,old?"g":"f")).name());params.add("prefix",textFact(field(p,old?"c":"b")));params.add("suffix",textFact(field(p,old?"d":"c")));row.add("parameters",params);}
                JsonArray members=new JsonArray();if(op==0||op==3||op==4)for(Object name:(Collection<?>)field(packet,old?"h":"k"))members.add((String)name);row.add("members",members);
            }else if(!old&&id==0x43){row.addProperty("remove",true);JsonArray ids=new JsonArray();for(Object u:(List<?>)field(packet,"b"))ids.add(uuid(u));row.add("removed",ids);}
            else{
                int flags;if(old){int op=((Enum<?>)field(packet,"a")).ordinal();row.addProperty("operation",op);flags=new int[]{1|4|16|32,4,16,32,0}[op];}
                else{flags=0;for(Object a:(Set<?>)field(packet,"b"))flags|=1<<((Enum<?>)a).ordinal();row.addProperty("flags",flags);}
                JsonArray entries=new JsonArray();for(Object entry:(List<?>)field(packet,old?"b":"c")){
                    Object profile=field(entry,old?"d":"b");JsonObject e=new JsonObject();e.add("uuid",uuid(old?call(profile,"getId"):field(entry,"a")));
                    if((flags&1)!=0){JsonObject p=new JsonObject();p.addProperty("name",(String)call(profile,old?"getName":"name"));JsonArray props=new JsonArray();for(Object prop:(Collection<?>)call(call(profile,old?"getProperties":"properties"),"values")){JsonObject property=new JsonObject();property.addProperty("name",(String)call(prop,old?"getName":"name"));property.addProperty("value",(String)call(prop,old?"getValue":"value"));String sig=(String)call(prop,old?"getSignature":"signature");property.add("signature",sig==null?JsonNull.INSTANCE:new JsonPrimitive(sig));props.add(property);}p.add("properties",props);e.add("profile",p);}
                    if((flags&4)!=0)e.addProperty("game_mode",(int)call(field(entry,old?"c":"e"),"a"));
                    if((flags&16)!=0)e.addProperty("latency",(int)field(entry,old?"b":"d"));
                    if((flags&32)!=0)e.add("display_name",textFact(field(entry,old?"e":"f")));
                    if(!old){
                        if((flags&8)!=0)e.addProperty("listed",(boolean)field(entry,"c"));
                        if((flags&64)!=0)e.addProperty("list_order",(int)field(entry,"h"));
                        if((flags&128)!=0)e.addProperty("show_hat",(boolean)field(entry,"g"));
                        if((flags&2)!=0){Object session=field(entry,"i");JsonElement info=JsonNull.INSTANCE;if(session!=null){JsonObject chat=new JsonObject();chat.add("uuid",uuid(field(session,"a")));Object key=field(session,"b");chat.addProperty("expires_at_epoch_millis",((Instant)field(key,"b")).toEpochMilli());chat.add("public_key",array(((PublicKey)field(key,"c")).getEncoded()));chat.add("key_signature",array((byte[])field(key,"d")));info=chat;}e.add("chat_session",info);}
                    }
                    entries.add(e);
                }row.add("entries",entries);
            }
            return row;
        }finally{((ByteBuf)read).release();((ByteBuf)write).release();}
    }
    static JsonObject team(int op,int visibility,int collision,int color,int options)throws Exception{
        ByteBuf b=Unpooled.buffer();try{string(b,"social");b.writeByte(op);if(op==0||op==2){text(b,"Team");b.writeByte(options);String[] names={"always","never","hideForOtherTeams","hideForOwnTeam"},rules={"always","never","pushOtherTeams","pushOwnTeam"};if(old){string(b,names[visibility]);string(b,rules[collision]);}else{integer(b,visibility);integer(b,collision);}integer(b,color);text(b,"Prefix雪");text(b,"Suffix");}if(op==0||op==3||op==4){integer(b,2);string(b,"SocialProbe");string(b,"OfflineHolder");}return readWrite("team",old?0x4c:0x6b,bytes(b));}finally{b.release();}
    }
    static JsonObject roster(int op,int flags,int mode,int latency,boolean optional)throws Exception{
        ByteBuf b=Unpooled.buffer();try{
            if(old)integer(b,op);else b.writeByte(flags);integer(b,1);uuid(b,0);
            int effective=old?new int[]{1|4|16|32,4,16,32,0}[op]:flags;
            if((effective&1)!=0){string(b,"SocialProbe");integer(b,2);string(b,"textures");string(b,"native-value");b.writeBoolean(true);string(b,"native-signature");string(b,"other");string(b,"other-value");b.writeBoolean(false);}
            if((effective&2)!=0){b.writeBoolean(optional);if(optional){uuid(b,16);b.writeLong(1840000000000L);integer(b,publicKey.length);b.writeBytes(publicKey);integer(b,3);b.writeBytes(new byte[]{9,8,7});}}
            if((effective&4)!=0)integer(b,mode);
            if((effective&8)!=0)b.writeBoolean(optional);
            if((effective&16)!=0)integer(b,latency);
            if((effective&32)!=0){b.writeBoolean(optional);if(optional)text(b,"Display雪");}
            if((effective&64)!=0)integer(b,300);
            if((effective&128)!=0)b.writeBoolean(optional);
            return readWrite("roster",old?0x33:0x44,bytes(b));
        }finally{b.release();}
    }
    static JsonObject removal()throws Exception{ByteBuf b=Unpooled.buffer();try{integer(b,1);uuid(b,0);return readWrite("roster",0x43,bytes(b));}finally{b.release();}}
    static JsonObject membership()throws Exception{
        Class<?> board=Class.forName(old?"dfm":"fur"),team=Class.forName(old?"dfk":"fum");Object state=board.getConstructor().newInstance();Method create=board.getMethod(old?"g":"c",String.class),join=board.getMethod("a",String.class,team),leave=board.getMethod("b",String.class,team);Object a=create.invoke(state,"a"),b=create.invoke(state,"b");join.invoke(state,"SocialProbe",a);String policy;try{Object duplicate=create.invoke(state,"a");if(duplicate!=a)throw new IllegalStateException("duplicate created another native team");policy="retained";}catch(InvocationTargetException e){if(e.getCause()instanceof IllegalArgumentException)policy="refused";else throw e;}boolean retained=((Collection<?>)call(a,old?"g":"h")).contains("SocialProbe");join.invoke(state,"SocialProbe",b);boolean moved=((Collection<?>)call(a,old?"g":"h")).isEmpty()&&((Collection<?>)call(b,old?"g":"h")).contains("SocialProbe");boolean refused=false;try{leave.invoke(state,"SocialProbe",a);}catch(InvocationTargetException e){if(e.getCause()instanceof IllegalStateException)refused=true;else throw e;}leave.invoke(state,"SocialProbe",b);JsonObject result=new JsonObject();result.addProperty("duplicate_create_policy",policy);result.addProperty("duplicate_attempt_keeps_members",retained);result.addProperty("joining_other_team_moves_holder",moved);result.addProperty("wrong_team_leave_refused",refused);result.addProperty("correct_leave_removes_holder",((Collection<?>)call(b,old?"g":"h")).isEmpty());if(!retained||!moved||!refused)throw new IllegalStateException("native membership assumptions differ");return result;
    }
    public static void main(String[]args)throws Exception{
        old=args[0].equals("1.16.1");if(!old){Class.forName("w").getMethod("a").invoke(null);Class.forName("amv").getMethod("a").invoke(null);registries=Class.forName("jr").getMethod("a",Class.forName("jq")).invoke(null,Class.forName("mi").getField("aR").get(null));SecureRandom random=SecureRandom.getInstance("SHA1PRNG");random.setSeed(new byte[]{21,11,16,1});KeyPairGenerator generator=KeyPairGenerator.getInstance("RSA");generator.initialize(2048,random);publicKey=generator.generateKeyPair().getPublic().getEncoded();}
        JsonObject out=new JsonObject();out.addProperty("version",args[0]);JsonArray rows=new JsonArray();for(int op=0;op<5;op++)rows.add(team(op,2,3,12,2));for(int color=0;color<16;color++)rows.add(team(2,0,0,color,3));rows.add(team(2,0,0,21,3));for(int v=0;v<4;v++)rows.add(team(2,v,0,21,3));for(int c=0;c<4;c++)rows.add(team(2,0,c,21,3));for(int f:new int[]{0,1,2,3,128})rows.add(team(2,0,0,21,f));
        if(old){for(int op=0;op<5;op++)rows.add(roster(op,0,1,300,true));for(int m=-1;m<4;m++)rows.add(roster(1,0,m,0,false));for(int n:new int[]{-1,0,300,Integer.MAX_VALUE})rows.add(roster(2,0,0,n,false));rows.add(roster(3,0,0,0,false));}
        else{for(int bit=0;bit<8;bit++)rows.add(roster(0,1<<bit,1,300,true));rows.add(roster(0,255,1,300,true));rows.add(roster(0,192,0,0,false));rows.add(roster(0,0,0,0,false));for(int m=0;m<4;m++)rows.add(roster(0,4,m,0,false));for(int n:new int[]{-1,0,300,Integer.MAX_VALUE})rows.add(roster(0,16,0,n,false));for(int flag:new int[]{2,8,32,128})rows.add(roster(0,flag,0,0,false));rows.add(removal());}
        out.add("packets",rows);out.add("native_membership_rules",membership());Files.writeString(Path.of(args[1]),new GsonBuilder().serializeNulls().setPrettyPrinting().create().toJson(out)+"\n");
    }
}
