package voxrig.oracle;
import java.lang.reflect.Method;
public class CapabilityEnums {
    public static void main(String[] arguments) throws Exception {
        for (String name : arguments) {
            Class<?> type = Class.forName(name);
            Object[] values = type.getEnumConstants();
            if (values == null) throw new IllegalArgumentException(name);
            for (Object value : values) {
                Enum<?> item = (Enum<?>) value;
                System.out.println(name + "\t" + item.ordinal() + "\t" + item.name());
            }
        }
    }
}
