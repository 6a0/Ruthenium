package net.ruthenium.gametest;

import com.mojang.blaze3d.vertex.VertexSorting;
import net.fabricmc.fabric.api.client.gametest.v1.context.ClientGameTestContext;
import net.minecraft.client.renderer.SectionBufferBuilderPack;
import net.minecraft.client.renderer.chunk.RenderRegionCache;
import net.minecraft.client.renderer.chunk.RenderSectionRegion;
import net.minecraft.client.renderer.chunk.SectionCompiler;
import net.minecraft.core.SectionPos;
import net.ruthenium.Initializer;
import net.ruthenium.config.Config;
import net.ruthenium.gametest.mixin.SectionRenderDispatcherAccessor;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import java.util.ArrayList;
import java.util.List;
import java.util.function.BiConsumer;

/**
 * {@code -Pbench.compileBench=true}: compiles the sections around the player on the render thread, one after
 * another, with the mesher switches all on, each one off, and all off, and logs the time per section (median of
 * {@link #ROUNDS} rounds per configuration, interleaved). Vanilla's tint caches are cleared before each round, as
 * F3+A does.
 */
final class CompileBenchmark {
    static final boolean ENABLED = Boolean.parseBoolean(BenchmarkClientGameTest.prop("compileBench", "false"));
    private static final Logger LOGGER = LoggerFactory.getLogger("Ruthenium Compile Benchmark");
    private static final int ROUNDS = 7;
    private static final boolean SCRATCH_AB = Boolean.parseBoolean(BenchmarkClientGameTest.prop("compileBenchScratchAb", "false"));
    private static final int SCRATCH_AB_RUNS = 6;

    private record Variant(String name, BiConsumer<Config, Boolean> apply) {
    }

    private CompileBenchmark() {
    }

    /**
     * The same sections compiled by 1 to all background-worker-count threads at once (each thread all sections, its
     * own regions and buffers), all switches as configured: wall time per section per thread shows how compiles
     * scale on this CPU apart from vanilla's scheduling. With {@code -Pbench.compileBenchScratchAb=true} each thread
     * count is measured {@link #SCRATCH_AB_RUNS} times, alternating the pooled compile scratch with the per-thread one
     * (switch {@code freeUnusedChunkData}), and the medians of both are logged.
     */
    private static void parallel(net.minecraft.client.Minecraft mc, SectionCompiler compiler, List<SectionPos> sections) {
        int max = net.minecraft.util.Util.maxAllowedExecutorThreads();
        Config config = Initializer.CONFIG;
        boolean saved = config.freeUnusedChunkData;
        for (int threads : new int[]{1, 2, 4, 8, 12, max}) {
            mc.level.clearTintCaches();
            List<List<RenderSectionRegion>> regions = new ArrayList<>();
            for (int t = 0; t < threads; t++) {
                RenderRegionCache cache = new RenderRegionCache();
                List<RenderSectionRegion> list = new ArrayList<>();
                for (SectionPos pos : sections) {
                    list.add(cache.createRegion(mc.level, pos.asLong()));
                }
                regions.add(list);
            }
            if (!SCRATCH_AB) {
                logParallel(threads, "", measureParallel(compiler, sections, regions, threads));
                continue;
            }
            double[] pooled = new double[SCRATCH_AB_RUNS];
            double[] perThread = new double[SCRATCH_AB_RUNS];
            try {
                for (int run = 0; run < SCRATCH_AB_RUNS; run++) {
                    // ABBA order
                    boolean pooledFirst = (run & 1) == 0;
                    for (int side = 0; side < 2; side++) {
                        boolean pool = (side == 0) == pooledFirst;
                        config.freeUnusedChunkData = pool;
                        (pool ? pooled : perThread)[run] = measureParallel(compiler, sections, regions, threads);
                    }
                }
            } finally {
                config.freeUnusedChunkData = saved;
            }
            java.util.Arrays.sort(pooled);
            java.util.Arrays.sort(perThread);
            logParallel(threads, " (pooled scratch, median of " + SCRATCH_AB_RUNS + ")", pooled[SCRATCH_AB_RUNS / 2]);
            logParallel(threads, " (per-thread scratch, median of " + SCRATCH_AB_RUNS + ")", perThread[SCRATCH_AB_RUNS / 2]);
        }
    }

    private static void logParallel(int threads, String what, double perSection) {
        LOGGER.info("Parallel compile, {} threads{}: {} us per section per thread, {} sections/s in total", threads, what,
                "%.1f".formatted(perSection), "%.0f".formatted(threads * 1.0e6 / perSection));
    }

    /** Wall time per section per thread of the slowest thread, in its last of 3 rounds */
    private static double measureParallel(SectionCompiler compiler, List<SectionPos> sections,
                                          List<List<RenderSectionRegion>> regions, int threads) {
        java.util.concurrent.CyclicBarrier barrier = new java.util.concurrent.CyclicBarrier(threads);
        long[] elapsed = new long[threads];
        List<Thread> workers = new ArrayList<>();
        for (int t = 0; t < threads; t++) {
            int thread = t;
            workers.add(Thread.ofPlatform().name("Compile-Bench-" + t).start(() -> {
                try (SectionBufferBuilderPack buffers = new SectionBufferBuilderPack()) {
                    for (int round = 0; round < 3; round++) {
                        barrier.await();
                        long start = System.nanoTime();
                        for (int i = 0; i < sections.size(); i++) {
                            SectionCompiler.Results results = compiler.compile(sections.get(i), regions.get(thread).get(i),
                                    VertexSorting.byDistance(0, 0, 0), buffers);
                            results.release();
                            buffers.clearAll();
                        }
                        elapsed[thread] = System.nanoTime() - start;
                    }
                } catch (Exception e) {
                    throw new RuntimeException(e);
                }
            }));
        }
        for (Thread worker : workers) {
            try {
                worker.join();
            } catch (InterruptedException e) {
                throw new RuntimeException(e);
            }
        }
        long slowest = java.util.Arrays.stream(elapsed).max().orElse(0);
        return slowest / 1000.0 / sections.size();
    }

    static void run(ClientGameTestContext context) {
        List<Variant> variants = List.of(
                new Variant("all on", (c, on) -> {
                }),
                new Variant("biomeBlendCache off", (c, on) -> c.biomeBlendCache = false),
                new Variant("sectionLightCache off", (c, on) -> c.sectionLightCache = false),
                new Variant("sectionStateCache off", (c, on) -> c.sectionStateCache = false),
                new Variant("terrainFaceCulling off", (c, on) -> c.terrainFaceCulling = false),
                // Per-thread compile scratch instead of the pooled one (SectionScratch)
                new Variant("freeUnusedChunkData off", (c, on) -> c.freeUnusedChunkData = false),
                new Variant("all off", (c, on) -> {
                    c.biomeBlendCache = false;
                    c.sectionLightCache = false;
                    c.sectionStateCache = false;
                    c.terrainFaceCulling = false;
                }));
        context.runOnClient(mc -> {
            SectionCompiler compiler = ((SectionRenderDispatcherAccessor) mc.levelRenderer.sectionRenderDispatcher()).ruthenium$getSectionCompiler();
            SectionPos center = SectionPos.of(mc.player.blockPosition());
            List<SectionPos> sections = new ArrayList<>();
            for (int dx = -4; dx <= 4; dx++) {
                for (int dz = -4; dz <= 4; dz++) {
                    for (int dy = -4; dy <= 1; dy++) {
                        sections.add(center.offset(dx, dy, dz));
                    }
                }
            }
            Config config = Initializer.CONFIG;
            boolean[] saved = {config.biomeBlendCache, config.sectionLightCache, config.sectionStateCache, config.terrainFaceCulling,
                    config.freeUnusedChunkData};
            long[][] nanos = new long[variants.size()][ROUNDS];
            try (SectionBufferBuilderPack buffers = new SectionBufferBuilderPack()) {
                for (int round = -2; round < ROUNDS; round++) {
                    for (int v = 0; v < variants.size(); v++) {
                        config.biomeBlendCache = saved[0];
                        config.sectionLightCache = saved[1];
                        config.sectionStateCache = saved[2];
                        config.terrainFaceCulling = saved[3];
                        config.freeUnusedChunkData = saved[4];
                        variants.get(v).apply().accept(config, false);
                        mc.level.clearTintCaches();
                        List<RenderSectionRegion> regions = new ArrayList<>();
                        RenderRegionCache cache = new RenderRegionCache();
                        for (SectionPos pos : sections) {
                            regions.add(cache.createRegion(mc.level, pos.asLong()));
                        }
                        long start = System.nanoTime();
                        for (int i = 0; i < sections.size(); i++) {
                            SectionCompiler.Results results = compiler.compile(sections.get(i), regions.get(i), VertexSorting.byDistance(0, 0, 0), buffers);
                            results.release();
                            buffers.clearAll();
                        }
                        long elapsed = System.nanoTime() - start;
                        if (round >= 0) {
                            nanos[v][round] = elapsed;
                        }
                    }
                }
            } finally {
                config.biomeBlendCache = saved[0];
                config.sectionLightCache = saved[1];
                config.sectionStateCache = saved[2];
                config.terrainFaceCulling = saved[3];
                config.freeUnusedChunkData = saved[4];
            }
            parallel(mc, compiler, sections);
            for (int v = 0; v < variants.size(); v++) {
                long[] times = nanos[v].clone();
                java.util.Arrays.sort(times);
                LOGGER.info("Compile {}: {} us per section (median of {} rounds of {} sections)", variants.get(v).name(),
                        "%.1f".formatted(times[ROUNDS / 2] / 1000.0 / sections.size()), ROUNDS, sections.size());
            }
        });
    }
}
