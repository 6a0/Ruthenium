package net.ruthenium.render;

import net.minecraft.client.Minecraft;
import net.ruthenium.Initializer;

import java.lang.management.GarbageCollectorMXBean;
import java.lang.management.ManagementFactory;
import java.util.Arrays;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.TreeMap;

/**
 * Diagnostic ({@code -Druthenium.frameStats=true}): every 10 s the log gets the average fps, the 1% low and what the
 * slowest 1% of frames spent their time on. A frame is one {@code Minecraft.runTick}, split into the swapchain acquire,
 * client ticks, the submit (which waits for the previous frame's GPU work), the present and everything else. Render
 * thread only.
 */
public final class FrameStats {
    public static final boolean ENABLED = Boolean.getBoolean("ruthenium.frameStats");

    public static final int ACQUIRE = 0, TICK = 1, SUBMIT = 2, PRESENT = 3;
    private static final String[] NAMES = {"acquire", "tick", "submit", "present", "other"};
    private static final int PARTS = 4;
    // Per frame: interval, the parts, GC flag
    private static final int STRIDE = PARTS + 2;
    private static final long WINDOW_NS = 10_000_000_000L;

    private static final List<GarbageCollectorMXBean> GCS = ManagementFactory.getGarbageCollectorMXBeans();

    private static long[] frames = new long[STRIDE * 16384];
    private static int count;
    private static final long[] current = new long[PARTS];
    private static final long[] started = new long[PARTS];
    private static long frameStart;
    private static long windowStart;
    private static long lastGcCount;

    private FrameStats() {
    }

    public static void begin(int part) {
        started[part] = System.nanoTime();
    }

    public static void end(int part) {
        current[part] += System.nanoTime() - started[part];
    }

    /** {@code runTick} HEAD: ends the previous frame */
    public static void frame() {
        long now = System.nanoTime();
        long gcCount = gcCount();
        if (frameStart != 0) {
            if ((count + 1) * STRIDE > frames.length) {
                frames = Arrays.copyOf(frames, frames.length * 2);
            }
            int o = count++ * STRIDE;
            frames[o] = now - frameStart;
            System.arraycopy(current, 0, frames, o + 1, PARTS);
            frames[o + PARTS + 1] = gcCount != lastGcCount ? 1 : 0;
        } else {
            windowStart = now;
        }
        lastGcCount = gcCount;
        Arrays.fill(current, 0);
        frameStart = now;

        if (now - windowStart >= WINDOW_NS && count > 0) {
            report(now - windowStart);
            count = 0;
            windowStart = now;
        }
    }

    private static long gcCount() {
        long sum = 0;
        for (GarbageCollectorMXBean gc : GCS) {
            sum += Math.max(0, gc.getCollectionCount());
        }
        return sum;
    }

    private static void report(long windowNs) {
        Integer[] order = new Integer[count];
        for (int i = 0; i < count; i++) {
            order[i] = i;
        }
        Arrays.sort(order, (a, b) -> Long.compare(frames[b * STRIDE], frames[a * STRIDE]));

        int slow = Math.max(1, count / 100);
        long slowSum = 0;
        long[] partSums = new long[PARTS + 1];
        int[] dominant = new int[PARTS + 1];
        int gcFrames = 0;
        int tickFrames = 0;
        Map<Double, Integer> bins = new TreeMap<>();
        for (int k = 0; k < slow; k++) {
            int o = order[k] * STRIDE;
            long interval = frames[o];
            slowSum += interval;
            long other = interval;
            int top = PARTS;
            long topNs = -1;
            for (int p = 0; p < PARTS; p++) {
                long ns = frames[o + 1 + p];
                partSums[p] += ns;
                other -= ns;
                if (ns > topNs) {
                    topNs = ns;
                    top = p;
                }
            }
            partSums[PARTS] += other;
            dominant[other > topNs ? PARTS : top]++;
            if (frames[o + PARTS + 1] != 0) {
                gcFrames++;
            }
            if (frames[o + 1 + TICK] != 0) {
                tickFrames++;
            }
            bins.merge(Math.round(interval / 250_000.0) * 0.25, 1, Integer::sum);
        }

        long p99 = frames[order[Math.max(0, (int) Math.ceil(count * 0.01) - 1)] * STRIDE];
        long median = frames[order[count / 2] * STRIDE];
        StringBuilder sb = new StringBuilder();
        sb.append(String.format(Locale.ROOT, "Frame stats: %d frames in %.1f s, avg %.0f fps, median %.2f ms; 1%% low %.0f fps (p99 %.2f ms), worst 1%% avg %.0f fps",
                count, windowNs / 1e9, count / (windowNs / 1e9), median / 1e6, 1e9 / p99, p99 / 1e6, 1e9 / (slowSum / (double) slow)));
        sb.append(String.format(Locale.ROOT, "%n  slowest %d frames, mean ms:", slow));
        for (int p = 0; p <= PARTS; p++) {
            sb.append(String.format(Locale.ROOT, " %s %.2f", NAMES[p], partSums[p] / 1e6 / slow));
        }
        sb.append("; largest part:");
        for (int p = 0; p <= PARTS; p++) {
            if (dominant[p] > 0) {
                sb.append(' ').append(NAMES[p]).append(' ').append(dominant[p]);
            }
        }
        sb.append(String.format(Locale.ROOT, "; with a tick %d, with a GC %d", tickFrames, gcFrames));
        sb.append("\n  their frame times (ms: frames):");
        bins.entrySet().stream()
                .sorted((a, b) -> b.getValue() - a.getValue())
                .limit(8)
                .forEach(e -> sb.append(String.format(Locale.ROOT, " %.2f: %d", e.getKey(), e.getValue())));
        Minecraft.getInstance().windowSurface().currentConfiguration()
                .ifPresent(c -> sb.append(String.format(Locale.ROOT, "%n  surface %dx%d %s", c.width(), c.height(), c.presentMode())));
        Initializer.LOGGER.info(sb.toString());
    }
}
