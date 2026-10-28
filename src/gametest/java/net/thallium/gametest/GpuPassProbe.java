package net.thallium.gametest;

import com.mojang.blaze3d.systems.RenderSystem;
import com.mojang.renderpearl.api.commands.GpuQueryPool;
import com.mojang.renderpearl.api.commands.RenderPass;

import java.util.OptionalLong;

/**
 * Diagnostic ({@code -Pbench.gpuFrame=true}, with {@link GpuFrameProbe}): GPU time of the main pass's solid part,
 * from timestamps written into the pass ({@code GpuPassProbeMixin}): solid terrain, solid features (entities, block
 * entities on the vanilla path, instanced parts), {@code RetainedBlockEntities} and {@code InstancedModels}.
 * Timestamps inside a pass are approximate (the GPU overlaps work around them). Render thread only.
 */
public final class GpuPassProbe {
    public static final String[] NAMES = {"solid terrain", "solid features", "retained block entities", "instanced models"};
    /** Timestamps per frame: start, after terrain, retained start, retained end, instanced start, instanced end */
    private static final int STAMPS = 6;
    private static final int FRAMES = 6;

    private static GpuQueryPool pool;
    private static final boolean[] complete = new boolean[FRAMES];
    private static final boolean[] stamped = new boolean[FRAMES * STAMPS];
    private static int frame;

    private static final long[] nanos = new long[NAMES.length];
    private static final long[] samples = new long[NAMES.length];

    private GpuPassProbe() {
    }

    /** Index: 0 start, 1 after terrain, 2/3 retained, 4/5 instanced */
    public static void stamp(RenderPass pass, int index) {
        if (!GpuFrameProbe.ENABLED) {
            return;
        }
        if (pool == null) {
            pool = RenderSystem.getDevice().createTimestampQueryPool(FRAMES * STAMPS);
        }
        if (index == 0) {
            frame++;
            collect(frame % FRAMES);
        }
        int slot = frame % FRAMES;
        pass.writeTimestamp(pool, slot * STAMPS + index);
        stamped[slot * STAMPS + index] = true;
        complete[slot] = true;
    }

    private static void collect(int slot) {
        if (!complete[slot]) {
            return;
        }
        complete[slot] = false;
        int base = slot * STAMPS;
        OptionalLong[] v = pool.getValues(base, STAMPS);
        add(0, v, base, 0, 1);
        add(1, v, base, 1, 2);
        add(2, v, base, 2, 3);
        add(3, v, base, 4, 5);
        for (int i = 0; i < STAMPS; i++) {
            stamped[base + i] = false;
        }
    }

    private static void add(int name, OptionalLong[] v, int base, int from, int to) {
        if (stamped[base + from] && stamped[base + to] && v[from].isPresent() && v[to].isPresent()) {
            nanos[name] += v[to].getAsLong() - v[from].getAsLong();
            samples[name]++;
        }
    }

    static void reset() {
        java.util.Arrays.fill(nanos, 0);
        java.util.Arrays.fill(samples, 0);
    }

    static String report() {
        StringBuilder sb = new StringBuilder("GPU main pass:");
        for (int i = 0; i < NAMES.length; i++) {
            sb.append(' ').append(NAMES[i]).append(' ')
                    .append("%.3f".formatted(samples[i] == 0 ? 0 : nanos[i] / 1.0e6 / samples[i])).append(" ms,");
        }
        return sb.toString();
    }
}
