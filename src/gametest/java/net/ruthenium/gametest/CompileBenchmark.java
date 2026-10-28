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

    private record Variant(String name, BiConsumer<Config, Boolean> apply) {
    }

    private CompileBenchmark() {
    }

    /**
     * The same sections compiled by 1 to all background-worker-count threads at once (each thread all sections, its
     * own regions and buffers), all switches as configured: wall time per section per thread shows how compiles
     * scale on this CPU apart from vanilla's scheduling.
     */
    private static void parallel(net.minecraft.client.Minecraft mc, SectionCompiler compiler, List<SectionPos> sections) {
        int max = net.minecraft.util.Util.maxAllowedExecutorThreads();
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
            double perSection = slowest / 1000.0 / sections.size();
            LOGGER.info("Parallel compile, {} threads: {} us per section per thread, {} sections/s in total", threads,
                    "%.1f".formatted(perSection), "%.0f".formatted(threads * 1.0e6 / perSection));
        }
    }

    static void run(ClientGameTestContext context) {
        List<Variant> variants = List.of(
                new Variant("all on", (c, on) -> {
                }),
                new Variant("biomeBlendCache off", (c, on) -> c.biomeBlendCache = false),
                new Variant("sectionLightCache off", (c, on) -> c.sectionLightCache = false),
                new Variant("sectionStateCache off", (c, on) -> c.sectionStateCache = false),
                new Variant("terrainFaceCulling off", (c, on) -> c.terrainFaceCulling = false),
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
            boolean[] saved = {config.biomeBlendCache, config.sectionLightCache, config.sectionStateCache, config.terrainFaceCulling};
            long[][] nanos = new long[variants.size()][ROUNDS];
            try (SectionBufferBuilderPack buffers = new SectionBufferBuilderPack()) {
                for (int round = -2; round < ROUNDS; round++) {
                    for (int v = 0; v < variants.size(); v++) {
                        config.biomeBlendCache = saved[0];
                        config.sectionLightCache = saved[1];
                        config.sectionStateCache = saved[2];
                        config.terrainFaceCulling = saved[3];
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
