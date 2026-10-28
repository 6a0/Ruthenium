package net.ruthenium.gametest;

import com.mojang.blaze3d.systems.RenderSystem;
import com.mojang.renderpearl.api.commands.GpuQueryPool;
import com.mojang.renderpearl.api.commands.RenderPass;
import net.minecraft.client.renderer.chunk.ChunkSectionLayer;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import java.util.OptionalLong;

/**
 * Diagnostic ({@code -Pbench.gpuProfile=true}): GPU time of each terrain layer's draws from timestamps around
 * {@code ChunkSectionsToRender.render}, and the quads drawn per layer ({@code TerrainDrawListMixin}). Results are
 * read {@link #FRAMES} frames later, so no frame waits for them. Render thread only.
 */
public final class GpuProbe {
    public static final boolean ENABLED = Boolean.parseBoolean(BenchmarkClientGameTest.prop("gpuProfile", "false"));
    private static final Logger LOGGER = LoggerFactory.getLogger("Ruthenium GPU Probe");

    private static final ChunkSectionLayer[] LAYERS = ChunkSectionLayer.values();
    private static final int FRAMES = 4;
    private static final int PER_FRAME = LAYERS.length * 2;

    private static GpuQueryPool pool;
    private static final boolean[] written = new boolean[FRAMES * PER_FRAME];
    private static int frame;
    private static ChunkSectionLayer lastLayer;

    private static final long[] gpuNanos = new long[LAYERS.length];
    private static final long[] samples = new long[LAYERS.length];
    private static final long[] quads = new long[LAYERS.length];
    private static final long[] draws = new long[LAYERS.length];
    private static long quadFrames;

    private GpuProbe() {
    }

    public static void beforeLayer(ChunkSectionLayer layer, RenderPass pass) {
        if (pool == null) {
            pool = RenderSystem.getDevice().createTimestampQueryPool(FRAMES * PER_FRAME);
        }
        if (lastLayer == null || layer.ordinal() <= lastLayer.ordinal()) {
            frame++;
            collect(frame % FRAMES);
        }
        lastLayer = layer;
        int index = (frame % FRAMES) * PER_FRAME + layer.ordinal() * 2;
        pass.writeTimestamp(pool, index);
    }

    public static void afterLayer(ChunkSectionLayer layer, RenderPass pass) {
        int index = (frame % FRAMES) * PER_FRAME + layer.ordinal() * 2 + 1;
        pass.writeTimestamp(pool, index);
        written[index - 1] = true;
        written[index] = true;
    }

    private static void collect(int slot) {
        for (ChunkSectionLayer layer : LAYERS) {
            int index = slot * PER_FRAME + layer.ordinal() * 2;
            if (!written[index]) {
                continue;
            }
            written[index] = written[index + 1] = false;
            OptionalLong[] values = pool.getValues(index, 2);
            if (values[0].isPresent() && values[1].isPresent()) {
                gpuNanos[layer.ordinal()] += values[1].getAsLong() - values[0].getAsLong();
                samples[layer.ordinal()]++;
            }
        }
    }

    public static void countDraws(ChunkSectionLayer layer, int[] commands, int count) {
        long indices = 0;
        for (int c = 0; c < count; c++) {
            indices += commands[c * 5];
        }
        quads[layer.ordinal()] += indices / 6;
        draws[layer.ordinal()] += count;
    }

    public static void countFrame() {
        quadFrames++;
    }

    static void reset() {
        java.util.Arrays.fill(gpuNanos, 0);
        java.util.Arrays.fill(samples, 0);
        java.util.Arrays.fill(quads, 0);
        java.util.Arrays.fill(draws, 0);
        quadFrames = 0;
    }

    static void report(String phase) {
        StringBuilder line = new StringBuilder("GPU ").append(phase).append(':');
        double total = 0;
        for (ChunkSectionLayer layer : LAYERS) {
            int l = layer.ordinal();
            double ms = samples[l] == 0 ? 0 : gpuNanos[l] / 1.0e6 / samples[l];
            total += ms;
            line.append(' ').append(layer.label()).append(' ').append("%.3f".formatted(ms)).append(" ms");
            if (quadFrames > 0) {
                line.append(" (").append(quads[l] / quadFrames).append(" quads, ")
                        .append(draws[l] / quadFrames).append(" draws)");
            }
            line.append(',');
        }
        line.append(" terrain ").append("%.3f".formatted(total)).append(" ms");
        line.append(", index type ").append(RenderSystem.getSequentialBuffer(
                com.mojang.renderpearl.api.pipeline.PrimitiveTopology.QUADS).type());
        LOGGER.info(line.toString());
        reportUnderground();
    }

    /** Quads (before face culling) of visible sections that are entirely below the surface of every column */
    private static void reportUnderground() {
        net.minecraft.client.Minecraft mc = net.minecraft.client.Minecraft.getInstance();
        long[] all = new long[LAYERS.length];
        long[] under = new long[LAYERS.length];
        int sections = 0, underSections = 0;
        for (var section : mc.levelRenderer.visibleSections()) {
            var origin = section.getRenderOrigin();
            var chunk = mc.level.getChunk(origin.getX() >> 4, origin.getZ() >> 4);
            int minSurface = Integer.MAX_VALUE;
            for (int x = 0; x < 16; x++) {
                for (int z = 0; z < 16; z++) {
                    minSurface = Math.min(minSurface, chunk.getHeight(net.minecraft.world.level.levelgen.Heightmap.Types.WORLD_SURFACE, x, z));
                }
            }
            boolean underground = origin.getY() + 16 <= minSurface - 1;
            sections++;
            if (underground) {
                underSections++;
            }
            var mesh = section.getSectionMesh();
            for (ChunkSectionLayer layer : LAYERS) {
                var draw = mesh.getSectionDraw(layer);
                if (draw != null) {
                    all[layer.ordinal()] += draw.indexCount() / 6;
                    if (underground) {
                        under[layer.ordinal()] += draw.indexCount() / 6;
                    }
                }
            }
        }
        StringBuilder line = new StringBuilder("Underground: ").append(underSections).append(" of ").append(sections).append(" sections");
        for (ChunkSectionLayer layer : LAYERS) {
            line.append(", ").append(layer.label()).append(' ').append(under[layer.ordinal()]).append(" of ").append(all[layer.ordinal()]).append(" quads");
        }
        LOGGER.info(line.toString());
    }
}
