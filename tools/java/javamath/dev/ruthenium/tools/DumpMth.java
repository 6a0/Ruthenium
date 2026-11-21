package dev.ruthenium.tools;

import java.io.IOException;
import java.lang.reflect.Field;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;
import java.util.Random;

import net.minecraft.util.Mth;

/**
 * Ruthenium developer tool. Run against the player's own installed client jar, it
 * records vanilla's sine table and the results of {@code Mth.sin}/{@code Mth.cos}
 * for many probe inputs, so ruthenium-javamath can be tested bit-for-bit.
 *
 * <p>Output (little-endian): magic "RJM1"; u32 table length; the table as f32 bits;
 * u32 probe count; per probe u64 input bits, u32 sin bits, u32 cos bits; then a
 * u32-length UTF-8 description of the JVM that produced it.
 *
 * <p>Usage: {@code java -cp <client.jar>:<libraries>:<classes> dev.ruthenium.tools.DumpMth <out>}
 */
public final class DumpMth {
    /** Table entries per radian, as vanilla defines it. */
    private static final double SCALE = 10430.378350470453;

    private DumpMth() {}

    public static void main(String[] args) throws Exception {
        if (args.length != 1) {
            System.err.println("usage: DumpMth <output file>");
            System.exit(2);
        }
        float[] table = readTable();
        double[] probes = probes(table.length);
        byte[] jvm = (System.getProperty("java.vm.name") + " " + System.getProperty("java.vm.version")
                + " " + System.getProperty("os.arch")).getBytes(StandardCharsets.UTF_8);

        ByteBuffer out = ByteBuffer.allocate(4 + 4 + table.length * 4 + 4 + probes.length * 16 + 4 + jvm.length)
                .order(ByteOrder.LITTLE_ENDIAN);
        out.put("RJM1".getBytes(StandardCharsets.US_ASCII));
        out.putInt(table.length);
        for (float f : table) {
            out.putInt(Float.floatToRawIntBits(f));
        }
        out.putInt(probes.length);
        for (double x : probes) {
            out.putLong(Double.doubleToRawLongBits(x));
            out.putInt(Float.floatToRawIntBits(Mth.sin(x)));
            out.putInt(Float.floatToRawIntBits(Mth.cos(x)));
        }
        out.putInt(jvm.length);
        out.put(jvm);

        Path path = Path.of(args[0]);
        if (path.getParent() != null) {
            Files.createDirectories(path.getParent());
        }
        Files.write(path, out.array());
        System.out.println("wrote " + table.length + " table entries and " + probes.length + " probes to " + path
                + " (" + new String(jvm, StandardCharsets.UTF_8) + ")");
    }

    /** Reads the private table itself: Mth.sin cannot address every entry exactly. */
    private static float[] readTable() throws ReflectiveOperationException, IOException {
        for (Field field : Mth.class.getDeclaredFields()) {
            if (field.getType() == float[].class && java.lang.reflect.Modifier.isStatic(field.getModifiers())) {
                field.setAccessible(true);
                float[] table = (float[]) field.get(null);
                if (table.length == 65536) {
                    return table.clone();
                }
            }
        }
        throw new IOException("no static float[65536] field found in Mth");
    }

    private static double[] probes(int tableLength) {
        List<Double> p = new ArrayList<>();
        double[] specials = {
            0.0, -0.0, 1.0, -1.0, Math.PI, -Math.PI, Math.PI / 2, -Math.PI / 2, 2 * Math.PI, 1e-300,
            Double.MIN_VALUE, -Double.MIN_VALUE, Double.MAX_VALUE, -Double.MAX_VALUE, Double.NaN,
            Double.POSITIVE_INFINITY, Double.NEGATIVE_INFINITY, 1e10, -1e10, 1e15, -1e15, 1e20, -1e20,
            (double) Long.MAX_VALUE, (double) Long.MIN_VALUE, 3.4028235e38, 1.0e-7, 359.99, 720.0,
        };
        for (double d : specials) {
            p.add(d);
        }
        // Every table boundary and its neighbours, positive and negative.
        for (int i = 0; i < tableLength; i++) {
            double x = i / SCALE;
            p.add(x);
            p.add(Math.nextDown(x));
            p.add(Math.nextUp(x));
            p.add(-x);
        }
        // Typical gameplay angles: yaw and pitch in degrees converted to radians,
        // as floats widened to double, plus large accumulated yaws.
        Random random = new Random(0x52754A4DL);
        for (int i = 0; i < 100_000; i++) {
            float degrees = (random.nextFloat() * 2 - 1) * 360f * (i % 10 == 0 ? 1000f : 1f);
            p.add((double) (degrees * (float) (Math.PI / 180.0)));
            p.add(random.nextGaussian() * 1e4);
        }
        double[] out = new double[p.size()];
        for (int i = 0; i < out.length; i++) {
            out[i] = p.get(i);
        }
        return out;
    }
}
