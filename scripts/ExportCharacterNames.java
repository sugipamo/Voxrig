// Owned observer of the running JDK's name lookup and relevant ROOT case folds.
import java.io.*;
import java.nio.file.*;
import java.util.*;
public final class ExportCharacterNames {
    static String quote(String s) {return "\""+s.replace("\\","\\\\").replace("\"","\\\"")+"\"";}
    public static void main(String[] args)throws Exception {
        int names=0,folds=0;
        try(BufferedWriter w=Files.newBufferedWriter(Path.of(args[0]))) {
            w.write("{\"java_version\":"+quote(System.getProperty("java.version"))+",\"names\":[");
            for(int cp=0;cp<=Character.MAX_CODE_POINT;cp++) {
                String name=Character.getName(cp);if(name==null)continue;
                if(!name.chars().allMatch(c->c<128)||Character.codePointOf(name)!=cp)throw new IllegalStateException("unreviewed native character name");
                if(names++!=0)w.write(",");w.write("["+cp+","+quote(name)+"]");
            }
            w.write("],\"ascii_upper\":[");
            for(int cp=128;cp<=Character.MAX_CODE_POINT;cp++) {
                String original=new String(Character.toChars(cp)),upper=original.toUpperCase(Locale.ROOT);
                if(!upper.chars().allMatch(c->c<128))continue;
                if(folds++!=0)w.write(",");w.write("["+cp+","+quote(upper)+"]");
            }
            w.write("]}\n");
        }
        System.out.println("original JDK names="+names+" ASCII ROOT folds="+folds);
    }
}
