package net.rubidium.gametest;

import net.minecraft.client.Minecraft;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.client.renderer.state.level.LevelRenderState;
import org.jspecify.annotations.Nullable;

import java.lang.management.GarbageCollectorMXBean;
import java.lang.management.ManagementFactory;
import java.util.Arrays;
import java.util.List;
import java.util.Locale;
import java.util.function.Consumer;

/**
 * Records frame-to-frame times and allocation counters on the client thread, and drives the camera pan.
 * All state is only touched from the client thread (directly from the frame hook, or via
 * {@code ClientGameTestContext.runOnClient}), so no synchronization is needed.
 */
public final class FrameRecorder {
    private static final int MAX_FRAMES = 1 << 20;
    private static final com.sun.management.ThreadMXBean THREAD_BEAN =
            (com.sun.management.ThreadMXBean) ManagementFactory.getThreadMXBean();

    private static final List<GarbageCollectorMXBean> GC_BEANS = ManagementFactory.getGarbageCollectorMXBeans();

    // Per-frame series; index i describes the frame that started at frame start i
    private static final long[] frameNanos = new long[MAX_FRAMES];
    private static final float[] frameYaw = new float[MAX_FRAMES];
    private static final int[] frameEntities = new int[MAX_FRAMES];
    private static final int[] frameBlockEntities = new int[MAX_FRAMES];
    private static final int[] frameGcCount = new int[MAX_FRAMES];
    /** Client ticks run between this frame's start and the previous one's (in the same loop iteration) */
    private static final byte[] frameTicks = new byte[MAX_FRAMES];
    private static int pendingTicks;
    private static float pendingYaw;
    private static int pendingEntities;
    private static int pendingBlockEntities;
    private static int frameCount;
    private static boolean recording;
    private static long lastFrameStart;
    private static long entitySum;
    private static long blockEntitySum;

    private static long phaseStartNanos;
    private static long phaseStartTotalAlloc;
    private static long phaseStartRenderThreadAlloc;
    private static long phaseStartGcCount;
    private static long phaseStartGcMillis;
    private static long phaseStartCpuNanos;
    /** Runs at every frame start, e.g. to follow section compilation per frame */
    private static @Nullable Consumer<Minecraft> frameWatcher;

    private static boolean panning;
    private static long panStartNanos;
    private static long panDurationNanos;
    private static float panStartYaw;
    private static float panPitch;

    private FrameRecorder() {
    }

    public static void onFrameStart(Minecraft minecraft) {
        long now = System.nanoTime();

        if (recording) {
            if (lastFrameStart != 0 && frameCount < MAX_FRAMES) {
                frameNanos[frameCount] = now - lastFrameStart;
                frameYaw[frameCount] = pendingYaw;
                frameEntities[frameCount] = pendingEntities;
                frameBlockEntities[frameCount] = pendingBlockEntities;
                frameGcCount[frameCount] = (int) (gcCount() - phaseStartGcCount);
                frameTicks[frameCount] = (byte) Math.min(pendingTicks, 127);
                frameCount++;
            }
            lastFrameStart = now;
            pendingEntities = 0;
            pendingBlockEntities = 0;
        }
        pendingTicks = 0;

        if (panning) {
            updatePan(minecraft, now);
        }

        if (frameWatcher != null) {
            frameWatcher.accept(minecraft);
        }

        if (recording && minecraft.player != null) {
            pendingYaw = minecraft.player.getYRot();
        }
    }

    public static void onTick() {
        pendingTicks++;
    }

    public static void onLevelExtracted(LevelRenderState levelState) {
        if (recording) {
            pendingEntities = levelState.entityRenderStates.size();
            pendingBlockEntities = levelState.blockEntityRenderStates.size();
            entitySum += pendingEntities;
            blockEntitySum += pendingBlockEntities;
        }
    }

    public static void startPhase() {
        frameCount = 0;
        lastFrameStart = 0;
        entitySum = 0;
        blockEntitySum = 0;
        recording = true;

        phaseStartNanos = System.nanoTime();
        phaseStartTotalAlloc = THREAD_BEAN.getTotalThreadAllocatedBytes();
        phaseStartRenderThreadAlloc = THREAD_BEAN.getCurrentThreadAllocatedBytes();
        phaseStartGcCount = gcCount();
        phaseStartGcMillis = gcMillis();
        phaseStartCpuNanos = THREAD_BEAN.getCurrentThreadCpuTime();
        if (LightProfiler.enabled()) {
            LightProfiler.INSTANCE.startRecording();
        }
        if (GpuFrameProbe.ENABLED) {
            GpuFrameProbe.reset();
            GpuPassProbe.reset();
        }
        TickProbe.reset();
        if (EntityCostProbe.ENABLED) {
            EntityCostProbe.reset();
        }
    }

    public static PhaseResult stopPhase(String name, int peakParticles) {
        recording = false;
        org.slf4j.LoggerFactory.getLogger("Rubidium Benchmark").info("{}: {}", name, tickSplit());
        org.slf4j.LoggerFactory.getLogger("Rubidium Benchmark").info("{}: {}", name, TickProbe.report());
        if (GpuFrameProbe.ENABLED) {
            org.slf4j.LoggerFactory.getLogger("Rubidium Benchmark").info("{}: {}; {}", name, GpuFrameProbe.report(), GpuPassProbe.report());
        }
        if (LightProfiler.enabled()) {
            org.slf4j.LoggerFactory.getLogger("Rubidium Benchmark").info("\n{}", LightProfiler.INSTANCE.stopRecording(name));
        }
        if (EntityCostProbe.ENABLED) {
            org.slf4j.LoggerFactory.getLogger("Rubidium Benchmark").info("\n{}", EntityCostProbe.report(name, frameCount, TickProbe.ticks));
        }

        long elapsed = System.nanoTime() - phaseStartNanos;
        long totalAlloc = THREAD_BEAN.getTotalThreadAllocatedBytes() - phaseStartTotalAlloc;
        long renderThreadAlloc = THREAD_BEAN.getCurrentThreadAllocatedBytes() - phaseStartRenderThreadAlloc;
        long gcCount = gcCount() - phaseStartGcCount;
        long gcMillis = gcMillis() - phaseStartGcMillis;
        // Includes time spent waiting inside the graphics driver if it spins instead of blocking
        long cpuNanos = THREAD_BEAN.getCurrentThreadCpuTime() - phaseStartCpuNanos;

        double avgEntities = frameCount > 0 ? (double) entitySum / frameCount : 0;
        double avgBlockEntities = frameCount > 0 ? (double) blockEntitySum / frameCount : 0;
        return PhaseResult.compute(name, Arrays.copyOf(frameNanos, frameCount), elapsed,
                totalAlloc, renderThreadAlloc, gcCount, gcMillis, peakParticles, avgEntities, avgBlockEntities, cpuNanos);
    }

    /**
     * Per-frame series of the last stopped phase as CSV: frame time, camera yaw, entities and block entities
     * extracted, and GCs since the phase started.
     */
    public static String framesCsv() {
        StringBuilder sb = new StringBuilder(frameCount * 32);
        sb.append("frame,ms,yaw,entities,blockEntities,gcs,ticks\n");
        for (int i = 0; i < frameCount; i++) {
            sb.append(i).append(',')
                    .append(String.format(Locale.ROOT, "%.4f", frameNanos[i] / 1.0e6)).append(',')
                    .append(String.format(Locale.ROOT, "%.2f", frameYaw[i])).append(',')
                    .append(frameEntities[i]).append(',')
                    .append(frameBlockEntities[i]).append(',')
                    .append(frameGcCount[i]).append(',')
                    .append(frameTicks[i]).append('\n');
        }
        return sb.toString();
    }

    /**
     * Frame times split by whether a client tick ran in the frame's interval: at ~1000 fps only ~2% of the frames
     * tick, so the tick's cost decides the p99.
     */
    private static String tickSplit() {
        long[] ticked = new long[frameCount];
        long[] plain = new long[frameCount];
        int t = 0, p = 0;
        for (int i = 0; i < frameCount; i++) {
            if (frameTicks[i] > 0) {
                ticked[t++] = frameNanos[i];
            } else {
                plain[p++] = frameNanos[i];
            }
        }
        return "frames with a tick: %d, %s; without: %d, %s".formatted(t, stats(Arrays.copyOf(ticked, t)), p,
                stats(Arrays.copyOf(plain, p)));
    }

    private static String stats(long[] nanos) {
        if (nanos.length == 0) {
            return "-";
        }
        Arrays.sort(nanos);
        long sum = 0;
        for (long n : nanos) {
            sum += n;
        }
        return String.format(Locale.ROOT, "avg %.3f ms, median %.3f, p99 %.3f", sum / 1.0e6 / nanos.length,
                nanos[nanos.length / 2] / 1.0e6, nanos[Math.min(nanos.length - 1, (int) (nanos.length * 0.99))] / 1.0e6);
    }

    public static void setFrameWatcher(@Nullable Consumer<Minecraft> watcher) {
        frameWatcher = watcher;
    }

    public static void startPan(Minecraft minecraft, float startYaw, double seconds) {
        startPan(minecraft, startYaw, 0.0f, seconds);
    }

    public static void startPan(Minecraft minecraft, float startYaw, float pitch, double seconds) {
        panStartYaw = startYaw;
        panPitch = pitch;
        panDurationNanos = (long) (seconds * 1.0e9);
        panStartNanos = System.nanoTime();
        panning = true;
        updatePan(minecraft, panStartNanos);
    }

    public static boolean isPanning() {
        return panning;
    }

    private static void updatePan(Minecraft minecraft, long now) {
        LocalPlayer player = minecraft.player;
        if (player == null) {
            panning = false;
            return;
        }

        double progress = (double) (now - panStartNanos) / panDurationNanos;
        if (progress >= 1.0) {
            progress = 1.0;
            panning = false;
        }

        float yaw = panStartYaw + (float) (360.0 * progress);
        player.setYRot(yaw);
        player.yRotO = yaw;
        player.setXRot(panPitch);
        player.xRotO = panPitch;
    }

    private static long gcCount() {
        long count = 0;
        for (GarbageCollectorMXBean bean : GC_BEANS) {
            count += Math.max(0, bean.getCollectionCount());
        }
        return count;
    }

    private static long gcMillis() {
        long millis = 0;
        for (GarbageCollectorMXBean bean : GC_BEANS) {
            millis += Math.max(0, bean.getCollectionTime());
        }
        return millis;
    }
}
