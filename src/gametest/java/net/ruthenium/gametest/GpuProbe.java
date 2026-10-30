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
 * {@code ChunkSectionsToRender.render} (both draws of a layer TerrainOcclusion draws twice), of TerrainOcclusion's
 * occluder pass and culling passes, and the quads per layer before the GPU's culling ({@code TerrainDrawListMixin}). Results are
 * read {@link #FRAMES} frames later, so no frame waits for them. Render thread only.
 */
public final class GpuProbe {
    public static final boolean ENABLED = Boolean.parseBoolean(BenchmarkClientGameTest.prop("gpuProfile", "false"));
    private static final Logger LOGGER = LoggerFactory.getLogger("Ruthenium GPU Probe");

    private static final ChunkSectionLayer[] LAYERS = ChunkSectionLayer.values();
    private static final int FRAMES = 4;
    /** A layer is drawn up to twice a frame: in TerrainOcclusion's occluder pass, then in the main pass */
    private static final int DRAWS = 2;
    private static final int LAYER_SLOTS = LAYERS.length * DRAWS * 2;
    /** TerrainOcclusion.execute: its start, after the occluder pass, the pyramid, the visibility pass, and its end */
    private static final int CULL_SLOTS = 5;
    private static final String[] CULL_SPANS = {"occluder pass", "Hi-Z", "visibility", "commands"};
    private static final int PER_FRAME = LAYER_SLOTS + CULL_SLOTS;

    private static GpuQueryPool pool;
    private static final boolean[] written = new boolean[FRAMES * PER_FRAME];
    private static int frame;
    private static final int[] drawsThisFrame = new int[LAYERS.length];
    private static final int[] lastDraw = new int[LAYERS.length];

    private static final long[][] gpuNanos = new long[LAYERS.length][DRAWS];
    private static final long[] samples = new long[LAYERS.length];
    private static final long[] cullNanos = new long[CULL_SLOTS - 1];
    private static long cullSamples;
    private static long visibleSections;
    private static long testedSections;
    private static final long[] quads = new long[LAYERS.length];
    private static final long[] draws = new long[LAYERS.length];
    private static long quadFrames;

    private GpuProbe() {
    }

    private static GpuQueryPool pool() {
        if (pool == null) {
            pool = RenderSystem.getDevice().createTimestampQueryPool(FRAMES * PER_FRAME);
        }
        return pool;
    }

    public static void beforeLayer(ChunkSectionLayer layer, RenderPass pass) {
        int l = layer.ordinal();
        int draw = Math.min(drawsThisFrame[l]++, DRAWS - 1);
        lastDraw[l] = draw;
        pass.writeTimestamp(pool(), (frame % FRAMES) * PER_FRAME + (l * DRAWS + draw) * 2);
    }

    public static void afterLayer(ChunkSectionLayer layer, RenderPass pass) {
        int index = (frame % FRAMES) * PER_FRAME + (layer.ordinal() * DRAWS + lastDraw[layer.ordinal()]) * 2 + 1;
        pass.writeTimestamp(pool(), index);
        written[index - 1] = true;
        written[index] = true;
    }

    /** A point of TerrainOcclusion.execute, in order ({@link #CULL_SLOTS}) */
    public static void cullTimestamp(int point) {
        int index = (frame % FRAMES) * PER_FRAME + LAYER_SLOTS + point;
        RenderSystem.getDevice().createCommandEncoder().writeTimestamp(pool(), index);
        written[index] = true;
    }

    private static void collect(int slot) {
        int base = slot * PER_FRAME;
        for (ChunkSectionLayer layer : LAYERS) {
            boolean any = false;
            for (int draw = 0; draw < DRAWS; draw++) {
                int index = base + (layer.ordinal() * DRAWS + draw) * 2;
                if (!written[index]) {
                    continue;
                }
                written[index] = written[index + 1] = false;
                OptionalLong[] values = pool.getValues(index, 2);
                if (values[0].isPresent() && values[1].isPresent()) {
                    gpuNanos[layer.ordinal()][draw] += values[1].getAsLong() - values[0].getAsLong();
                    any = true;
                }
            }
            if (any) {
                samples[layer.ordinal()]++;
            }
        }
        int cull = base + LAYER_SLOTS;
        boolean all = true;
        for (int i = 0; i < CULL_SLOTS; i++) {
            all &= written[cull + i];
            written[cull + i] = false;
        }
        if (all) {
            OptionalLong[] values = pool.getValues(cull, CULL_SLOTS);
            boolean present = true;
            for (OptionalLong value : values) {
                present &= value.isPresent();
            }
            if (present) {
                for (int i = 0; i < CULL_SLOTS - 1; i++) {
                    cullNanos[i] += values[i + 1].getAsLong() - values[i].getAsLong();
                }
                cullSamples++;
            }
        }
    }

    /** A section's visibility as TerrainOcclusion read it back */
    public static void countVisibility(boolean visible) {
        testedSections++;
        if (visible) {
            visibleSections++;
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

    /** Once per frame, before its terrain is drawn (TerrainDrawList.writeCommands) */
    public static void countFrame() {
        quadFrames++;
        if (pool != null) {
            frame++;
            collect(frame % FRAMES);
        }
        java.util.Arrays.fill(drawsThisFrame, 0);
    }

    static void reset() {
        for (long[] layer : gpuNanos) {
            java.util.Arrays.fill(layer, 0);
        }
        java.util.Arrays.fill(samples, 0);
        java.util.Arrays.fill(cullNanos, 0);
        cullSamples = 0;
        visibleSections = 0;
        testedSections = 0;
        java.util.Arrays.fill(quads, 0);
        java.util.Arrays.fill(draws, 0);
        quadFrames = 0;
    }

    static void report(String phase) {
        StringBuilder line = new StringBuilder("GPU ").append(phase).append(':');
        double total = 0;
        for (ChunkSectionLayer layer : LAYERS) {
            int l = layer.ordinal();
            double ms = samples[l] == 0 ? 0 : (gpuNanos[l][0] + gpuNanos[l][1]) / 1.0e6 / samples[l];
            total += ms;
            line.append(' ').append(layer.label()).append(' ').append("%.3f".formatted(ms)).append(" ms");
            if (gpuNanos[l][1] != 0) {
                line.append(" (").append("%.3f".formatted(gpuNanos[l][0] / 1.0e6 / samples[l])).append(" occluders)");
            }
            if (quadFrames > 0) {
                line.append(" (").append(quads[l] / quadFrames).append(" quads, ")
                        .append(draws[l] / quadFrames).append(" draws)");
            }
            line.append(',');
        }
        if (cullSamples > 0) {
            for (int i = 0; i < CULL_SPANS.length; i++) {
                double ms = cullNanos[i] / 1.0e6 / cullSamples;
                line.append(' ').append(CULL_SPANS[i]).append(' ').append("%.3f".formatted(ms)).append(" ms,");
                if (i > 0) {
                    total += ms;
                }
            }
        }
        if (testedSections > 0) {
            line.append(" sections visible ").append("%.1f".formatted(100.0 * visibleSections / testedSections)).append("%,");
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
