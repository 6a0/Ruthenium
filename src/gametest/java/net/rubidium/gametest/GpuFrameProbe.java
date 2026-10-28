package net.rubidium.gametest;

import com.mojang.blaze3d.systems.RenderSystem;
import com.mojang.renderpearl.api.commands.GpuQueryPool;

import java.util.OptionalLong;

/**
 * Diagnostic ({@code -Pbench.gpuFrame=true}): the GPU time of each frame's command buffer, from a timestamp at the
 * start of {@code Minecraft.renderFrame} (the first command after the previous submit) to one just before the frame's
 * submit. Commands only run once submitted, so the interval is the GPU's work on the frame, without the time it
 * waited for the CPU. Results are read {@link #FRAMES} frames later, so nothing waits for them. Render thread only.
 */
public final class GpuFrameProbe {
    public static final boolean ENABLED = Boolean.parseBoolean(BenchmarkClientGameTest.prop("gpuFrame", "false"));
    private static final int FRAMES = 6;

    private static GpuQueryPool pool;
    private static final boolean[] written = new boolean[FRAMES];
    private static int frame;
    private static boolean started;

    private static long gpuNanos;
    private static long samples;
    private static long maxNanos;

    private GpuFrameProbe() {
    }

    public static void frameStart() {
        if (pool == null) {
            pool = RenderSystem.getDevice().createTimestampQueryPool(FRAMES * 2);
        }
        frame++;
        int slot = frame % FRAMES;
        if (written[slot]) {
            written[slot] = false;
            OptionalLong[] values = pool.getValues(slot * 2, 2);
            if (values[0].isPresent() && values[1].isPresent()) {
                long nanos = values[1].getAsLong() - values[0].getAsLong();
                gpuNanos += nanos;
                maxNanos = Math.max(maxNanos, nanos);
                samples++;
            }
        }
        RenderSystem.getDevice().createCommandEncoder().writeTimestamp(pool, slot * 2);
        started = true;
    }

    public static void beforeSubmit() {
        if (!started) {
            return;
        }
        int slot = frame % FRAMES;
        RenderSystem.getDevice().createCommandEncoder().writeTimestamp(pool, slot * 2 + 1);
        written[slot] = true;
        started = false;
    }

    static void reset() {
        gpuNanos = 0;
        samples = 0;
        maxNanos = 0;
    }

    static String report() {
        return "GPU frame %.3f ms avg (max %.3f, %d frames)".formatted(samples == 0 ? 0 : gpuNanos / 1.0e6 / samples,
                maxNanos / 1.0e6, samples);
    }
}
