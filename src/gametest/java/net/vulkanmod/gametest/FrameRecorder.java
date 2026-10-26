package net.vulkanmod.gametest;

import net.minecraft.client.Minecraft;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.client.renderer.state.level.LevelRenderState;

import java.lang.management.GarbageCollectorMXBean;
import java.lang.management.ManagementFactory;
import java.util.Arrays;
import java.util.List;
import java.util.Locale;

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

    private static boolean panning;
    private static long panStartNanos;
    private static long panDurationNanos;
    private static float panStartYaw;

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
                frameCount++;
            }
            lastFrameStart = now;
            pendingEntities = 0;
            pendingBlockEntities = 0;
        }

        if (panning) {
            updatePan(minecraft, now);
        }

        if (recording && minecraft.player != null) {
            pendingYaw = minecraft.player.getYRot();
        }
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
    }

    public static PhaseResult stopPhase(String name, int peakParticles) {
        recording = false;

        long elapsed = System.nanoTime() - phaseStartNanos;
        long totalAlloc = THREAD_BEAN.getTotalThreadAllocatedBytes() - phaseStartTotalAlloc;
        long renderThreadAlloc = THREAD_BEAN.getCurrentThreadAllocatedBytes() - phaseStartRenderThreadAlloc;
        long gcCount = gcCount() - phaseStartGcCount;
        long gcMillis = gcMillis() - phaseStartGcMillis;

        double avgEntities = frameCount > 0 ? (double) entitySum / frameCount : 0;
        double avgBlockEntities = frameCount > 0 ? (double) blockEntitySum / frameCount : 0;
        return PhaseResult.compute(name, Arrays.copyOf(frameNanos, frameCount), elapsed,
                totalAlloc, renderThreadAlloc, gcCount, gcMillis, peakParticles, avgEntities, avgBlockEntities);
    }

    /**
     * Per-frame series of the last stopped phase as CSV: frame time, camera yaw, entities and block entities
     * extracted, and GCs since the phase started.
     */
    public static String framesCsv() {
        StringBuilder sb = new StringBuilder(frameCount * 32);
        sb.append("frame,ms,yaw,entities,blockEntities,gcs\n");
        for (int i = 0; i < frameCount; i++) {
            sb.append(i).append(',')
                    .append(String.format(Locale.ROOT, "%.4f", frameNanos[i] / 1.0e6)).append(',')
                    .append(String.format(Locale.ROOT, "%.2f", frameYaw[i])).append(',')
                    .append(frameEntities[i]).append(',')
                    .append(frameBlockEntities[i]).append(',')
                    .append(frameGcCount[i]).append('\n');
        }
        return sb.toString();
    }

    public static void startPan(Minecraft minecraft, float startYaw, double seconds) {
        panStartYaw = startYaw;
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
        player.setXRot(0.0f);
        player.xRotO = 0.0f;
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
