package net.ruthenium.gametest;

import com.mojang.blaze3d.systems.RenderSystem;
import com.mojang.renderpearl.api.commands.GpuQueryPool;
import com.mojang.renderpearl.api.commands.RenderPass;
import org.lwjgl.system.MemoryStack;
import org.lwjgl.vulkan.EXTCalibratedTimestamps;
import org.lwjgl.vulkan.VkCalibratedTimestampInfoEXT;
import org.lwjgl.vulkan.VkDevice;

import java.lang.reflect.Field;
import java.nio.LongBuffer;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.OptionalLong;

/**
 * Diagnostic ({@code -Pbench.gpuTimeline=frame|passes}): the frame's timeline on the GPU and the CPU. GPU timestamps
 * at the start of {@code renderFrame}, before the swapchain blit and before the submit ({@code frame}), and with
 * {@code passes} also around every render pass and at points inside the main pass. Each segment between consecutive
 * marks is averaged per phase. CPU events ({@code System.nanoTime}): acquire, blit, submit, {@code vkQueueSubmit2},
 * the wait for the previous submit, present. On Vulkan with calibrated timestamps (device and QPC domains) the GPU
 * marks are placed on the CPU clock: submit to GPU start (when the GPU was idle) and GPU end to the CPU's wakeup.
 * Results are read {@link #FRAMES} frames later. Render thread only.
 */
public final class GpuTimelineProbe {
    private static final String MODE = BenchmarkClientGameTest.prop("gpuTimeline", "off");
    public static final boolean ENABLED = !MODE.equals("off");
    public static final boolean PASSES = MODE.equals("passes");
    private static final int FRAMES = 8;
    private static final int MARKS = 96;

    public static final int C_FRAME = 0, C_ACQ0 = 1, C_ACQ1 = 2, C_BLIT = 3, C_SUBMIT = 4, C_QS0 = 5, C_QS1 = 6,
            C_AWAIT0 = 7, C_AWAIT1 = 8, C_PRESENT0 = 9, C_PRESENT1 = 10, C_COUNT = 11;
    private static final String[] C_NAMES = {"frame", "acquire", "acquired", "blit", "submit", "queueSubmit",
            "queueSubmitted", "await", "awaited", "present", "presented"};

    private static GpuQueryPool pool;
    private static final String[][] labels = new String[FRAMES][MARKS];
    private static final int[] counts = new int[FRAMES];
    private static final long[][] cpu = new long[FRAMES][C_COUNT];
    private static final long[] offsets = new long[FRAMES];
    private static long frame;
    private static int slot;
    private static boolean inFrame;
    private static boolean recording;

    // Previous collected frame (for gaps and the wakeup after its end)
    private static long prevFrameId = -10;
    private static long prevGpuStart, prevGpuEnd;
    private static long[] prevCpu = new long[C_COUNT];
    private static long prevOffset;

    private static final Map<String, long[]> segments = new LinkedHashMap<>();
    private static final List<double[]> perFrame = new ArrayList<>();
    private static long missing;

    // Calibration
    private static VkDevice vkDevice;
    private static boolean calibrationTried;
    private static long calibratedOffset = Long.MIN_VALUE;
    private static long lastCalibration;
    private static double qpcToNanos = 0;

    private GpuTimelineProbe() {
    }

    public static void cpu(int event) {
        if (inFrame) {
            cpu[slot][event] = System.nanoTime();
        }
    }

    /** {@code renderFrame} HEAD */
    public static void frameStart() {
        if (pool == null) {
            pool = RenderSystem.getDevice().createTimestampQueryPool(FRAMES * MARKS);
        }
        frame++;
        slot = (int) (frame % FRAMES);
        if (counts[slot] > 0) {
            collect(frame - FRAMES, slot);
        }
        counts[slot] = 0;
        Arrays.fill(cpu[slot], 0);
        calibrate();
        offsets[slot] = calibratedOffset;
        inFrame = true;
        cpu[slot][C_FRAME] = System.nanoTime();
        mark("frame");
    }

    /** After the submit call returned (the next marks belong to the next frame) */
    public static void frameEnd() {
        inFrame = false;
    }

    public static void mark(String label) {
        if (!inFrame || counts[slot] >= MARKS) {
            return;
        }
        int i = counts[slot]++;
        labels[slot][i] = label;
        RenderSystem.getDevice().createCommandEncoder().writeTimestamp(pool, slot * MARKS + i);
    }

    public static void markPass(RenderPass pass, String label) {
        if (!inFrame || !PASSES || counts[slot] >= MARKS) {
            return;
        }
        int i = counts[slot]++;
        labels[slot][i] = label;
        pass.writeTimestamp(pool, slot * MARKS + i);
    }

    public static void markOutside(String label) {
        if (PASSES) {
            mark(label);
        }
    }

    private static void collect(long id, int s) {
        int n = counts[s];
        OptionalLong[] v = pool.getValues(s * MARKS, n);
        for (OptionalLong value : v) {
            if (value.isEmpty()) {
                missing++;
                return;
            }
        }
        if (recording) {
            for (int i = 1; i < n; i++) {
                String key = labels[s][i - 1] + " -> " + labels[s][i];
                long[] acc = segments.computeIfAbsent(key, k -> new long[2]);
                acc[0] += v[i].getAsLong() - v[i - 1].getAsLong();
                acc[1]++;
            }
        }
        long gpuStart = v[0].getAsLong();
        long gpuEnd = v[n - 1].getAsLong();
        long[] c = cpu[s];
        long offset = offsets[s];
        if (recording && prevFrameId == id - 1) {
            // period, busy, gap, cpu frame, acquire, submit->(qs) , await, present, latency, wakeup
            double period = gpuStart - prevGpuStart;
            double busy = gpuEnd - gpuStart;
            double gap = gpuStart - prevGpuEnd;
            double cpuFrame = c[C_FRAME] - prevCpu[C_FRAME];
            double acquire = c[C_ACQ1] - c[C_ACQ0];
            double queueSubmit = c[C_QS1] - c[C_QS0];
            double await = c[C_AWAIT1] - c[C_AWAIT0];
            double present = c[C_PRESENT1] - c[C_PRESENT0];
            double record = c[C_SUBMIT] - c[C_FRAME];
            double latency = Double.NaN, wakeup = Double.NaN, idleAtSubmit = Double.NaN, endToQs = Double.NaN;
            if (offset != Long.MIN_VALUE && prevOffset != Long.MIN_VALUE) {
                // GPU start of this frame on the CPU clock, against this frame's vkQueueSubmit2 call
                long startCpu = gpuStart + offset;
                long prevEndCpu = prevGpuEnd + prevOffset;
                idleAtSubmit = prevEndCpu < c[C_QS0] ? 1 : 0;
                latency = startCpu - Math.max(c[C_QS0], prevEndCpu);
                // The previous frame's end against the await that waited for it (this frame's await)
                if (c[C_AWAIT0] < prevEndCpu) {
                    wakeup = c[C_AWAIT1] - prevEndCpu;
                }
                endToQs = c[C_QS0] - prevEndCpu;
            }
            perFrame.add(new double[]{period, busy, gap, cpuFrame, acquire, queueSubmit, await, present, record,
                    latency, wakeup, idleAtSubmit, endToQs});
        }
        prevFrameId = id;
        prevGpuStart = gpuStart;
        prevGpuEnd = gpuEnd;
        prevCpu = c.clone();
        prevOffset = offset;
    }

    /** Device timestamp to {@code System.nanoTime} offset through VK_EXT_calibrated_timestamps (device and QPC domains) */
    private static void calibrate() {
        if (frame - lastCalibration < 32 && calibratedOffset != Long.MIN_VALUE) {
            return;
        }
        lastCalibration = frame;
        if (!calibrationTried) {
            calibrationTried = true;
            try {
                Object device = RenderSystem.getDevice();
                Object backend = field(device, "backend");
                if (backend != null && backend.getClass().getSimpleName().equals("VulkanDevice")) {
                    vkDevice = (VkDevice) backend.getClass().getMethod("vkDevice").invoke(backend);
                }
                // QPC ticks to nanoTime: Java's nanoTime on Windows is QPC scaled to ns
                long f = queryFrequency();
                qpcToNanos = f > 0 ? 1.0e9 / f : 0;
            } catch (Throwable t) {
                org.slf4j.LoggerFactory.getLogger("Ruthenium GPU Timeline").warn("No calibration: {}", t.toString());
                vkDevice = null;
            }
        }
        if (vkDevice == null || qpcToNanos == 0) {
            return;
        }
        try (MemoryStack stack = MemoryStack.stackPush()) {
            VkCalibratedTimestampInfoEXT.Buffer infos = VkCalibratedTimestampInfoEXT.calloc(2, stack);
            infos.get(0).sType$Default().timeDomain(EXTCalibratedTimestamps.VK_TIME_DOMAIN_DEVICE_EXT);
            infos.get(1).sType$Default().timeDomain(EXTCalibratedTimestamps.VK_TIME_DOMAIN_QUERY_PERFORMANCE_COUNTER_EXT);
            LongBuffer values = stack.callocLong(2);
            LongBuffer deviation = stack.callocLong(1);
            int result = EXTCalibratedTimestamps.vkGetCalibratedTimestampsEXT(vkDevice, infos, values, deviation);
            if (result == 0) {
                long hostNanos = Math.round(values.get(1) * qpcToNanos);
                calibratedOffset = hostNanos - values.get(0);
            }
        } catch (Throwable t) {
            vkDevice = null;
        }
    }

    private static long queryFrequency() throws Throwable {
        var linker = java.lang.foreign.Linker.nativeLinker();
        var kernel = java.lang.foreign.SymbolLookup.libraryLookup("kernel32", java.lang.foreign.Arena.global());
        var fn = linker.downcallHandle(kernel.find("QueryPerformanceFrequency").orElseThrow(),
                java.lang.foreign.FunctionDescriptor.of(java.lang.foreign.ValueLayout.JAVA_INT, java.lang.foreign.ValueLayout.ADDRESS));
        try (var arena = java.lang.foreign.Arena.ofConfined()) {
            var out = arena.allocate(java.lang.foreign.ValueLayout.JAVA_LONG);
            int ok = (int) fn.invoke(out);
            return ok != 0 ? out.get(java.lang.foreign.ValueLayout.JAVA_LONG, 0) : 0;
        }
    }

    private static Object field(Object o, String name) throws Exception {
        for (Class<?> c = o.getClass(); c != null; c = c.getSuperclass()) {
            try {
                Field f = c.getDeclaredField(name);
                f.setAccessible(true);
                return f.get(o);
            } catch (NoSuchFieldException ignored) {
            }
        }
        return null;
    }

    static void reset() {
        segments.clear();
        perFrame.clear();
        missing = 0;
        recording = true;
    }

    static String report(String phase) {
        StringBuilder sb = new StringBuilder("GPU timeline of ").append(phase).append(": ").append(perFrame.size())
                .append(" frame pairs, ").append(missing).append(" incomplete, calibration ")
                .append(calibratedOffset == Long.MIN_VALUE ? "none" : "QPC").append('\n');
        String[] names = {"GPU period (start to start)", "GPU busy (frame mark to submit mark)", "GPU gap (submit mark to next start)",
                "CPU frame (renderFrame to renderFrame)", "CPU acquire", "CPU vkQueueSubmit2", "CPU await previous submit",
                "CPU present", "CPU renderFrame HEAD to submit", "submit to GPU start (GPU idle or busy until)",
                "previous GPU end to await return (CPU was waiting)", "GPU idle at vkQueueSubmit2 (fraction)", "previous GPU end to vkQueueSubmit2"};
        for (int k = 0; k < names.length; k++) {
            double[] values = new double[perFrame.size()];
            int m = 0;
            for (double[] f : perFrame) {
                if (!Double.isNaN(f[k])) {
                    values[m++] = f[k];
                }
            }
            if (m == 0) {
                continue;
            }
            values = Arrays.copyOf(values, m);
            double mean = Arrays.stream(values).average().orElse(0);
            Arrays.sort(values);
            if (k == 11) {
                sb.append("  %-48s %.3f%n".formatted(names[k], mean));
            } else {
                sb.append("  %-48s mean %8.1f us  median %8.1f  p10 %8.1f  p90 %8.1f%n".formatted(names[k], mean / 1e3,
                        values[m / 2] / 1e3, values[m / 10] / 1e3, values[m * 9 / 10] / 1e3));
            }
        }
        sb.append("  segments (mean us over the frames that have them, frames):\n");
        long frames = 0;
        for (long[] acc : segments.values()) {
            frames = Math.max(frames, acc[1]);
        }
        for (Map.Entry<String, long[]> e : segments.entrySet()) {
            long[] acc = e.getValue();
            sb.append("    %-70s %8.1f  %6.3f per frame  (%d)%n".formatted(e.getKey(), acc[0] / 1e3 / acc[1],
                    frames == 0 ? 0 : acc[0] / 1e3 / frames, acc[1]));
        }
        recording = false;
        return sb.toString();
    }
}
