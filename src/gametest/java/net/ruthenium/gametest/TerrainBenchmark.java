package net.ruthenium.gametest;

import net.fabricmc.fabric.api.client.gametest.v1.context.ClientGameTestContext;
import net.fabricmc.fabric.api.client.gametest.v1.context.TestSingleplayerContext;
import net.fabricmc.fabric.impl.client.gametest.world.TestWorldSaveImpl;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.screens.worldselection.WorldCreationUiState;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.renderer.chunk.CompiledSectionMesh;
import net.minecraft.world.level.gamerules.GameRules;
import net.minecraft.world.level.levelgen.Heightmap;
import org.apache.commons.io.FileUtils;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

/**
 * Singleplayer terrain benchmark ({@code -Pbench.scene=terrain}).
 * <p>
 * The world is generated once from a fixed seed into {@code build/run/bench/worlds/} and copied into the run's saves
 * before every run, so every configuration loads the same chunks from disk. The player is a spectator floating
 * {@code bench.terrainHeight} blocks above the highest block at the spot, looking slightly down, so the view
 * reaches the render distance.
 * <ol>
 *     <li>Joining: until all chunks are loaded and compiled (not measured: in singleplayer the chunks arrive as fast as
 *     the integrated server loads them, ~60 per second here; {@code loadSeconds} of the result).</li>
 *     <li>An untimed pan, so sections in every direction are compiled before measuring.</li>
 *     <li>"static": {@code bench.staticSeconds} at yaw {@code bench.staticYaw}.</li>
 *     <li>"pan": a full 360 degree yaw pan over {@code bench.panSeconds}.</li>
 *     <li>"rebuild": {@code bench.terrainRebuilds} times, all sections are invalidated like F3+A does, and the time
 *     until the visible ones are compiled again is measured; {@code rebuildSectionsPerSecond} (the median) is the
 *     client's section compile throughput (see {@link #rebuild} for why not the time).</li>
 * </ol>
 */
final class TerrainBenchmark {
    private static final Logger LOGGER = LoggerFactory.getLogger("Ruthenium Terrain Benchmark");
    private static final int TICKS_PER_SECOND = 20;

    private static final String SEED = BenchmarkClientGameTest.prop("terrainSeed", "8279265");
    private static final String WORLD_NAME = "terrain-" + SEED;
    private static final int RENDER_DISTANCE = Integer.parseInt(BenchmarkClientGameTest.prop("terrainRenderDistance", "32"));
    /** Spot of the benchmark; empty means the world spawn */
    private static final String SPOT = BenchmarkClientGameTest.prop("terrainSpot", "");
    private static final int HEIGHT = Integer.parseInt(BenchmarkClientGameTest.prop("terrainHeight", "16"));
    private static final float PITCH = Float.parseFloat(BenchmarkClientGameTest.prop("terrainPitch", "15"));
    private static final String[] TERRAIN_SHOT_YAWS = BenchmarkClientGameTest.prop("terrainShotYaws", "90,180,270").split(",");
    /** Switches Ruthenium's compressed terrain vertices off and on again through the settings, with screenshots */
    private static final boolean SWITCH_CHECK = Boolean.parseBoolean(BenchmarkClientGameTest.prop("terrainSwitchCheck", "false"));
    private static final int REBUILDS = Integer.parseInt(BenchmarkClientGameTest.prop("terrainRebuilds", "3"));
    private static final double LOAD_TIMEOUT_SECONDS = 120;

    private TerrainBenchmark() {
    }

    static void run(ClientGameTestContext context, String label, String runId) {
        context.runOnClient(mc -> mc.options.renderDistance().set(RENDER_DISTANCE));

        Path template = Path.of("..", "worlds", WORLD_NAME);
        Path save = context.computeOnClient(mc -> mc.getLevelSource().getBaseDir().resolve(WORLD_NAME));
        if (!Files.isDirectory(template)) {
            generateTemplate(context, template, save);
        }
        try {
            FileUtils.deleteDirectory(save.toFile());
            FileUtils.copyDirectory(template.toFile(), save.toFile());
        } catch (IOException e) {
            throw new RuntimeException("Failed to copy the terrain world", e);
        }

        List<PhaseResult> results = new ArrayList<>();
        Map<String, String> frameSeries = new HashMap<>();

        long joinStart = System.nanoTime();
        try (TestSingleplayerContext world = new TestWorldSaveImpl(context, save).open()) {
            // The saved player already floats at the spot (see generateTemplate)
            double loadSeconds = waitForTerrain(context);
            double joinSeconds = (System.nanoTime() - joinStart) / 1.0e9;
            LOGGER.info("Terrain loaded in {} s", "%.2f".formatted(loadSeconds));

            if (BiomeTintCheck.ENABLED) {
                BiomeTintCheck.run(context);
            }
            if (MeshCompareCheck.ENABLED) {
                MeshCompareCheck.run(context);
            }
            if (CompileBenchmark.ENABLED) {
                CompileBenchmark.run(context);
            }

            // Compile what the other directions see before measuring
            pan(context, 0.0f, 8.0);
            waitForCompiles(context);

            for (String yaw : TERRAIN_SHOT_YAWS) {
                setView(context, Float.parseFloat(yaw.trim()));
                context.takeScreenshot(label + "-terrain-yaw" + yaw.trim() + "-" + runId);
                if (OcclusionProbe.ENABLED) {
                    OcclusionProbe.survey(context, "yaw" + yaw.trim());
                }
            }

            setView(context, BenchmarkClientGameTest.STATIC_YAW);
            if (SWITCH_CHECK) {
                CompressedTerrainSwitchCheck.run(context, label, runId);
            }
            context.takeScreenshot(label + "-terrain-static-" + runId);
            int renderedSections = context.computeOnClient(mc -> mc.levelExtractor.countRenderedSections());
            context.runOnClient(_ -> {
                FrameRecorder.startPhase();
                GpuProbe.reset();
            });
            BenchmarkClientGameTest.waitSeconds(context, BenchmarkClientGameTest.STATIC_SECONDS);
            results.add(context.computeOnClient(_ -> FrameRecorder.stopPhase("static", -1)));
            if (GpuProbe.ENABLED) {
                context.runOnClient(_ -> GpuProbe.report("static"));
            }
            LOGGER.info(results.getLast().summaryLine());
            if (OcclusionProbe.ENABLED) {
                OcclusionProbe.run(context, label, runId, results);
            }

            context.runOnClient(_ -> {
                FrameRecorder.startPhase();
                GpuProbe.reset();
            });
            pan(context, BenchmarkClientGameTest.STATIC_YAW, BenchmarkClientGameTest.PAN_SECONDS);
            results.add(context.computeOnClient(_ -> FrameRecorder.stopPhase("pan", -1)));
            if (GpuProbe.ENABLED) {
                context.runOnClient(_ -> GpuProbe.report("pan"));
            }
            frameSeries.put("pan", context.computeOnClient(_ -> FrameRecorder.framesCsv()));
            LOGGER.info(results.getLast().summaryLine());

            setView(context, BenchmarkClientGameTest.STATIC_YAW);
            double[] rebuildSeconds = new double[REBUILDS];
            double[] rebuildRates = new double[REBUILDS];
            context.runOnClient(_ -> FrameRecorder.startPhase());
            for (int i = 0; i < REBUILDS; i++) {
                double[] rebuild = rebuild(context);
                rebuildSeconds[i] = rebuild[0];
                rebuildRates[i] = rebuild[1] / rebuild[0];
                LOGGER.info("Rebuild {}: {} sections in {} s, {} per second", i + 1, (int) rebuild[1],
                        "%.3f".formatted(rebuild[0]), "%.0f".formatted(rebuildRates[i]));
            }
            results.add(context.computeOnClient(_ -> FrameRecorder.stopPhase("rebuild", -1)));
            LOGGER.info(results.getLast().summaryLine());
            if (REBUILDS > 0) {
                // Recompiled sections fade in again (chunk fade-in time)
                context.waitTicks(3 * TICKS_PER_SECOND);
                int visible = context.computeOnClient(mc -> mc.levelRenderer.visibleSections().size());
                LOGGER.info("Visible sections after the rebuilds: {}", visible);
                context.takeScreenshot(label + "-terrain-rebuilt-" + runId);
            }
            Arrays.sort(rebuildSeconds);
            Arrays.sort(rebuildRates);

            var environment = context.computeOnClient(BenchmarkClientGameTest::describeEnvironment);
            environment.addProperty("label", label + "-terrain");
            environment.addProperty("seed", SEED);
            environment.addProperty("terrainHeight", HEIGHT);
            environment.addProperty("pitch", PITCH);
            environment.addProperty("loadSeconds", loadSeconds);
            environment.addProperty("joinSeconds", joinSeconds);
            environment.addProperty("renderedSections", renderedSections);
            if (REBUILDS > 0) {
                environment.addProperty("rebuildSeconds", rebuildSeconds[REBUILDS / 2]);
                environment.addProperty("rebuildSectionsPerSecond", rebuildRates[REBUILDS / 2]);
            }
            BenchmarkClientGameTest.writeResults(label + "-terrain", results, environment, frameSeries);
        }
    }

    /**
     * Creates the world, moves the player to the spot, waits until the terrain around it is generated, and keeps
     * the saved world as the template of all runs.
     */
    private static void generateTemplate(ClientGameTestContext context, Path template, Path save) {
        LOGGER.info("Generating the terrain world (seed {}) into {}", SEED, template.toAbsolutePath());
        try {
            FileUtils.deleteDirectory(save.toFile());
        } catch (IOException e) {
            throw new RuntimeException(e);
        }

        try (TestSingleplayerContext world = context.worldBuilder()
                .setUseConsistentSettings(false)
                .adjustSettings(creator -> {
                    creator.setName(WORLD_NAME);
                    creator.setSeed(SEED);
                    creator.setGameMode(WorldCreationUiState.SelectedGameMode.CREATIVE);
                    creator.getGameRules().set(GameRules.ADVANCE_TIME, false, null);
                    creator.getGameRules().set(GameRules.ADVANCE_WEATHER, false, null);
                    creator.getGameRules().set(GameRules.SPAWN_MOBS, false, null);
                })
                .create()) {
            if (!world.getWorldSave().getSaveDirectory().getFileName().toString().equals(WORLD_NAME)) {
                throw new AssertionError("World was saved as " + world.getWorldSave().getSaveDirectory());
            }

            world.getServer().runCommand("time set 6000");
            world.getServer().runCommand("weather clear");
            world.getServer().runCommand("gamemode spectator @a");
            waitForTerrain(context);

            int[] spot = context.computeOnClient(mc -> {
                int x, z;
                if (SPOT.isBlank()) {
                    x = mc.player.getBlockX();
                    z = mc.player.getBlockZ();
                } else {
                    String[] parts = SPOT.split(",");
                    x = Integer.parseInt(parts[0].trim());
                    z = Integer.parseInt(parts[1].trim());
                }
                ClientLevel level = mc.level;
                return new int[]{x, level.getHeight(Heightmap.Types.MOTION_BLOCKING, x, z), z};
            });
            String tp = "tp @a %d.5 %d %d.5 %s %s".formatted(spot[0], spot[1] + HEIGHT, spot[2],
                    BenchmarkClientGameTest.STATIC_YAW, PITCH);
            LOGGER.info("Benchmark spot: /{}", tp);
            world.getServer().runCommand(tp);
            context.waitTicks(10);
            waitForTerrain(context);
            pan(context, 0.0f, 8.0);
            waitForCompiles(context);
            setView(context, BenchmarkClientGameTest.STATIC_YAW);
        }

        try {
            FileUtils.copyDirectory(save.toFile(), template.toFile());
        } catch (IOException e) {
            throw new RuntimeException("Failed to keep the terrain world", e);
        }
    }

    /** Waits for all chunks to arrive and to be compiled; returns the seconds waited */
    private static double waitForTerrain(ClientGameTestContext context) {
        long start = System.nanoTime();
        int[] lastChunks = {-1};
        int[] stableTicks = {0};
        int[] ticks = {0};
        context.waitFor(mc -> {
            if (mc.level == null || mc.player == null || mc.gui.screen() != null) {
                return false;
            }
            int chunks = mc.level.getChunkSource().getLoadedChunksCount();
            boolean compiled = mc.levelRenderer.hasRenderedAllSections();
            if (chunks == lastChunks[0] && compiled && chunks >= expectedChunks()) {
                stableTicks[0]++;
            } else {
                stableTicks[0] = 0;
            }
            lastChunks[0] = chunks;
            if (++ticks[0] % TICKS_PER_SECOND == 0) {
                var dispatcher = mc.levelRenderer.sectionRenderDispatcher();
                LOGGER.info("Loading: {} s, {} chunks, {} sections queued, {} fps", ticks[0] / TICKS_PER_SECOND, chunks,
                        dispatcher != null ? dispatcher.getCompileQueueSize() : -1, mc.getFps());
            }
            return stableTicks[0] >= TICKS_PER_SECOND;
        }, (int) (LOAD_TIMEOUT_SECONDS * TICKS_PER_SECOND));
        // Minus the second of stability that confirmed it
        return (System.nanoTime() - start) / 1.0e9 - 1.0;
    }

    /**
     * Invalidates all compiled sections and waits until the visible ones are compiled again; returns the seconds
     * from the invalidation to the frame where the last section had been compiled, and the sections compiled.
     * <p>
     * After such a reset with a still camera, vanilla's occlusion graph often misses sections for good (seen with
     * F3+A: ~1770 instead of ~2080 visible here, the hillside on the left missing), and they only come back with the
     * next full graph update. So the throughput (sections per second) is the comparable value, and the graph is
     * rebuilt afterwards (untimed) to leave the scene complete.
     */
    private static double[] rebuild(ClientGameTestContext context) {
        // Followed every frame: the compiled count and when it last changed
        int[] last = {-1};
        long[] lastChange = {0};
        long workerCpuStart = workerCpuNanos();
        long start = context.computeOnClient(mc -> {
            mc.levelExtractor.allChanged();
            long now = System.nanoTime();
            lastChange[0] = now;
            FrameRecorder.setFrameWatcher(m -> {
                int compiled = countCompiledVisible(m);
                if (compiled != last[0]) {
                    last[0] = compiled;
                    lastChange[0] = System.nanoTime();
                }
            });
            return now;
        });
        int compiled;
        try {
            compiled = waitUntilCompiled(context);
        } finally {
            context.runOnClient(_ -> FrameRecorder.setFrameWatcher(null));
        }
        double seconds = context.computeOnClient(_ -> lastChange[0] - start) / 1.0e9;
        double workerCpu = (workerCpuNanos() - workerCpuStart) / 1.0e9;
        LOGGER.info("Rebuild worker CPU: {} s in {} s ({} threads busy on average)", "%.3f".formatted(workerCpu),
                "%.3f".formatted(seconds), "%.1f".formatted(workerCpu / seconds));
        completeOcclusionGraph(context);
        return new double[]{seconds, compiled};
    }

    /** CPU time of vanilla's background workers (section compiles run there) */
    private static long workerCpuNanos() {
        java.lang.management.ThreadMXBean threads = java.lang.management.ManagementFactory.getThreadMXBean();
        long total = 0;
        for (java.lang.management.ThreadInfo info : threads.getThreadInfo(threads.getAllThreadIds())) {
            if (info != null && info.getThreadName().startsWith("Worker-Main")) {
                long cpu = threads.getThreadCpuTime(info.getThreadId());
                if (cpu > 0) {
                    total += cpu;
                }
            }
        }
        return total;
    }

    /** Waits until every visible section is compiled again after the terrain was reset, with a complete graph */
    static void waitForVisibleCompiled(ClientGameTestContext context) {
        waitUntilCompiled(context);
        completeOcclusionGraph(context);
    }

    /** Rebuilds the occlusion graph (as moving the camera far enough would) and waits for what it adds */
    private static void completeOcclusionGraph(ClientGameTestContext context) {
        context.runOnClient(mc -> mc.levelRenderer.sectionOcclusionGraph().invalidate());
        context.waitTicks(5);
        int visible = waitUntilCompiled(context);
        LOGGER.info("{} visible sections after rebuilding the occlusion graph", visible);
    }

    /** Waits until the visible sections are compiled and the compile queue has stayed empty; returns their count */
    private static int waitUntilCompiled(ClientGameTestContext context) {
        int[] stableTicks = {0};
        int[] ticks = {0};
        int[] visible = {0};
        context.waitFor(mc -> {
            visible[0] = mc.levelRenderer.visibleSections().size();
            int compiledVisible = countCompiledVisible(mc);
            if (++ticks[0] % TICKS_PER_SECOND == 0) {
                var dispatcher = mc.levelRenderer.sectionRenderDispatcher();
                LOGGER.info("Compiling: {} of {} visible sections compiled, {} queued", compiledVisible, visible[0],
                        dispatcher != null ? dispatcher.getCompileQueueSize() : -1);
            }
            boolean compiled = visible[0] > 100 && compiledVisible == visible[0] && mc.levelRenderer.hasRenderedAllSections();
            stableTicks[0] = compiled ? stableTicks[0] + 1 : 0;
            return stableTicks[0] >= TICKS_PER_SECOND / 2;
        }, 30 * TICKS_PER_SECOND);
        return visible[0];
    }

    /** Visible sections that have been compiled (including empty ones) */
    private static int countCompiledVisible(Minecraft mc) {
        int compiled = 0;
        for (var section : mc.levelRenderer.visibleSections()) {
            if (section.getSectionMesh() != CompiledSectionMesh.UNCOMPILED) {
                compiled++;
            }
        }
        return compiled;
    }

    private static void waitForCompiles(ClientGameTestContext context) {
        int[] stableTicks = {0};
        context.waitFor(mc -> {
            stableTicks[0] = mc.levelRenderer.hasRenderedAllSections() ? stableTicks[0] + 1 : 0;
            return stableTicks[0] >= TICKS_PER_SECOND / 2;
        }, (int) (LOAD_TIMEOUT_SECONDS * TICKS_PER_SECOND));
    }

    /** Chunks within the render distance (a disc), minus a margin for the edge */
    private static int expectedChunks() {
        double r = RENDER_DISTANCE;
        return (int) (Math.PI * r * r * 0.9);
    }

    private static void pan(ClientGameTestContext context, float startYaw, double seconds) {
        context.runOnClient(mc -> FrameRecorder.startPan(mc, startYaw, PITCH, seconds));
        context.waitFor(_ -> !FrameRecorder.isPanning(), (int) (seconds * TICKS_PER_SECOND * 10));
    }

    private static void setView(ClientGameTestContext context, float yaw) {
        context.runOnClient(mc -> {
            mc.player.setYRot(yaw);
            mc.player.yRotO = yaw;
            mc.player.setXRot(PITCH);
            mc.player.xRotO = PITCH;
        });
        context.waitTicks(10);
        waitForCompiles(context);
    }
}
