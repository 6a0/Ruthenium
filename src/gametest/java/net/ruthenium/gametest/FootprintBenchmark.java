package net.ruthenium.gametest;

import com.google.gson.GsonBuilder;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import net.fabricmc.fabric.api.client.gametest.v1.context.ClientGameTestContext;
import net.fabricmc.fabric.api.client.gametest.v1.context.TestSingleplayerContext;
import net.fabricmc.fabric.impl.client.gametest.world.TestWorldSaveImpl;
import net.minecraft.client.gui.screens.TitleScreen;
import org.apache.commons.io.FileUtils;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import javax.management.ObjectName;
import java.io.IOException;
import java.lang.foreign.Arena;
import java.lang.foreign.FunctionDescriptor;
import java.lang.foreign.Linker;
import java.lang.foreign.MemorySegment;
import java.lang.foreign.SymbolLookup;
import java.lang.foreign.ValueLayout;
import java.lang.invoke.MethodHandle;
import java.lang.management.BufferPoolMXBean;
import java.lang.management.ManagementFactory;
import java.lang.management.MemoryPoolMXBean;
import java.lang.management.MemoryType;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.time.LocalDateTime;
import java.time.format.DateTimeFormatter;
import java.util.Arrays;
import java.util.concurrent.CompletableFuture;

/**
 * Memory and load time benchmark ({@code -Pbench.scene=footprint}). Everything it reports is either close to
 * deterministic (live heap after a full GC, class histogram, GPU allocations) or repeated within the launch (reloads,
 * joins), so one launch per configuration is enough; a second only checks the startup time, which is one sample.
 * <ol>
 *     <li>Startup: JVM uptime when the test first sees the title screen with no loading overlay.</li>
 *     <li>Snapshot "title".</li>
 *     <li>{@code bench.footprintReloads} resource reloads (F3+T), each timed from {@code reloadResourcePacks} to its
 *     future completing; snapshot "reloaded" (growth over "title" is what reloads keep).</li>
 *     <li>{@code bench.footprintJoins} joins of the terrain scene's world (a fresh copy each time) at render distance
 *     {@code bench.footprintRenderDistance}: seconds until the world is open (player in, no screen) and until all
 *     chunks are loaded and compiled. Snapshot "world" after the first join, "left" after leaving the last.</li>
 * </ol>
 * A snapshot runs a full GC through the class histogram diagnostic command (written to
 * {@code results/footprint/<label>-<time>-histo-<snapshot>.txt}), then records the live heap, committed heap
 * (before the GC, i.e. what the game had grown to), metaspace, code cache, direct buffers, the process's working set
 * and private bytes, and its dedicated and shared GPU memory (Windows performance counters, summed over adapters).
 * With {@code -PbenchNmt} the JVM's native memory summary is written next to the histograms.
 */
final class FootprintBenchmark {
    private static final Logger LOGGER = LoggerFactory.getLogger("Ruthenium Footprint Benchmark");
    private static final int TICKS_PER_SECOND = 20;
    private static final double MB = 1024.0 * 1024.0;

    private static final int RELOADS = Integer.parseInt(BenchmarkClientGameTest.prop("footprintReloads", "3"));
    private static final int JOINS = Integer.parseInt(BenchmarkClientGameTest.prop("footprintJoins", "2"));
    private static final int RENDER_DISTANCE = Integer.parseInt(BenchmarkClientGameTest.prop("footprintRenderDistance", "12"));
    private static final String WORLD_NAME = "terrain-" + BenchmarkClientGameTest.prop("terrainSeed", "8279265");
    /** Rows of each class histogram kept in the files (the full one is thousands of classes) */
    private static final int HISTOGRAM_ROWS = 80;
    /** Snapshot to also write a heap dump of (live objects) for {@code scripts/HprofPath.java}; empty for none */
    private static final String HEAP_DUMP = BenchmarkClientGameTest.prop("footprintHeapDump", "");

    private FootprintBenchmark() {
    }

    static void run(ClientGameTestContext context, String label, String runId) {
        context.waitFor(mc -> mc.gui.overlay() == null && mc.gui.screen() instanceof TitleScreen, 120 * TICKS_PER_SECOND);
        double startupSeconds = ManagementFactory.getRuntimeMXBean().getUptime() / 1000.0;
        LOGGER.info("Title screen after {} s", "%.2f".formatted(startupSeconds));

        Path outDir = Path.of(BenchmarkClientGameTest.prop("outDir", "../results"));
        Path histoDir = outDir.resolve("footprint");
        String prefix = label + "-footprint-" + runId;
        try {
            Files.createDirectories(histoDir);
        } catch (IOException e) {
            throw new RuntimeException(e);
        }

        JsonObject snapshots = new JsonObject();
        snapshots.add("title", snapshot(context, "title", histoDir, prefix));

        double[] reloadSeconds = new double[RELOADS];
        for (int i = 0; i < RELOADS; i++) {
            reloadSeconds[i] = reload(context);
            LOGGER.info("Reload {}: {} s", i + 1, "%.3f".formatted(reloadSeconds[i]));
        }
        if (RELOADS > 0) {
            snapshots.add("reloaded", snapshot(context, "reloaded", histoDir, prefix));
        }

        int previousDistance = context.computeOnClient(mc -> mc.options.renderDistance().get());
        context.runOnClient(mc -> mc.options.renderDistance().set(RENDER_DISTANCE));
        Path template = Path.of("..", "worlds", WORLD_NAME);
        if (JOINS > 0 && !Files.isDirectory(template)) {
            throw new AssertionError("No terrain world at " + template.toAbsolutePath() + "; run the terrain scene once to generate it");
        }
        Path save = context.computeOnClient(mc -> mc.getLevelSource().getBaseDir().resolve(WORLD_NAME));
        double[] openSeconds = new double[JOINS];
        double[] loadedSeconds = new double[JOINS];
        int[] chunks = new int[JOINS];
        for (int i = 0; i < JOINS; i++) {
            try {
                FileUtils.deleteDirectory(save.toFile());
                FileUtils.copyDirectory(template.toFile(), save.toFile());
            } catch (IOException e) {
                throw new RuntimeException("Failed to copy the terrain world", e);
            }
            long start = System.nanoTime();
            try (TestSingleplayerContext ignored = new TestWorldSaveImpl(context, save).open()) {
                context.waitFor(mc -> mc.level != null && mc.player != null && mc.gui.screen() == null, 120 * TICKS_PER_SECOND);
                openSeconds[i] = (System.nanoTime() - start) / 1.0e9;
                chunks[i] = waitForChunks(context);
                loadedSeconds[i] = (System.nanoTime() - start) / 1.0e9 - 1.0;
                LOGGER.info("Join {}: open after {} s, {} chunks loaded and compiled after {} s", i + 1,
                        "%.2f".formatted(openSeconds[i]), chunks[i], "%.2f".formatted(loadedSeconds[i]));
                if (i == 0) {
                    BenchmarkClientGameTest.waitSeconds(context, 3);
                    snapshots.add("world", snapshot(context, "world", histoDir, prefix));
                }
            }
        }
        context.runOnClient(mc -> mc.options.renderDistance().set(previousDistance));
        if (JOINS > 0) {
            context.waitFor(mc -> mc.level == null && mc.gui.screen() instanceof TitleScreen, 60 * TICKS_PER_SECOND);
            context.waitTicks(TICKS_PER_SECOND);
            snapshots.add("left", snapshot(context, "left", histoDir, prefix));
        }

        JsonObject environment = context.computeOnClient(mc -> {
            JsonObject env = new JsonObject();
            var device = com.mojang.blaze3d.systems.RenderSystem.getDevice().getDeviceInfo();
            env.addProperty("label", label + "-footprint");
            env.addProperty("backend", device.backendName());
            env.addProperty("gpu", device.name());
            env.addProperty("ruthenium", net.fabricmc.loader.api.FabricLoader.getInstance().isModLoaded("ruthenium"));
            env.addProperty("sodium", net.fabricmc.loader.api.FabricLoader.getInstance().isModLoaded("sodium"));
            env.addProperty("java", System.getProperty("java.vm.name") + " " + System.getProperty("java.version"));
            env.addProperty("maxHeapMB", Runtime.getRuntime().maxMemory() / (1024 * 1024));
            env.addProperty("renderDistance", RENDER_DISTANCE);
            env.addProperty("world", WORLD_NAME);
            return env;
        });

        JsonObject root = new JsonObject();
        root.add("environment", environment);
        root.addProperty("startupSeconds", startupSeconds);
        root.add("reloadSeconds", array(reloadSeconds));
        root.add("joinOpenSeconds", array(openSeconds));
        root.add("joinLoadedSeconds", array(loadedSeconds));
        JsonArray chunkArray = new JsonArray();
        Arrays.stream(chunks).forEach(chunkArray::add);
        root.add("joinChunks", chunkArray);
        root.add("snapshots", snapshots);

        StringBuilder summary = new StringBuilder();
        summary.append("== Footprint: ").append(label).append(" ==\n");
        summary.append("startup %.2f s\n".formatted(startupSeconds));
        summary.append("reload median %.3f s %s\n".formatted(median(reloadSeconds), Arrays.toString(reloadSeconds)));
        // The first join is cold (the integrated server's chunk loading isn't compiled yet; ~10x slower), so no median
        summary.append("join open %s s, loaded and compiled %s s (first cold, then warm), chunks %s\n".formatted(
                Arrays.toString(openSeconds), Arrays.toString(loadedSeconds), Arrays.toString(chunks)));
        for (String name : snapshots.keySet()) {
            JsonObject s = snapshots.getAsJsonObject(name);
            summary.append("%-8s live heap %7.1f MB, committed %7.1f MB, metaspace %6.1f MB, code %5.1f MB, direct %6.1f MB, working set %7.1f MB, private %7.1f MB, GPU dedicated %7.1f MB, shared %6.1f MB\n"
                    .formatted(name, mb(s, "liveHeap"), mb(s, "committedHeapBeforeGc"), mb(s, "metaspace"), mb(s, "codeCache"),
                            mb(s, "directBuffers"), mb(s, "workingSet"), mb(s, "privateBytes"), mb(s, "gpuDedicated"), mb(s, "gpuShared")));
        }
        LOGGER.info("\n{}", summary);
        try {
            String timestamp = LocalDateTime.now().format(DateTimeFormatter.ofPattern("yyyyMMdd-HHmmss"));
            String json = new GsonBuilder().setPrettyPrinting().create().toJson(root);
            Files.writeString(outDir.resolve(label + "-footprint-" + timestamp + ".json"), json);
            Files.writeString(outDir.resolve(label + "-footprint-" + timestamp + ".txt"), summary);
        } catch (IOException e) {
            throw new RuntimeException("Failed to write footprint results", e);
        }
    }

    /** Reloads resources as F3+T does; returns the seconds until the reload's future completes */
    private static double reload(ClientGameTestContext context) {
        long[] times = new long[2];
        CompletableFuture<?> future = context.computeOnClient(mc -> {
            times[0] = System.nanoTime();
            return mc.reloadResourcePacks().whenComplete((_, _) -> times[1] = System.nanoTime());
        });
        context.waitFor(mc -> future.isDone() && mc.gui.overlay() == null, 120 * TICKS_PER_SECOND);
        future.join();
        context.waitTicks(TICKS_PER_SECOND);
        return (times[1] - times[0]) / 1.0e9;
    }

    /** Waits until the chunks in range stop arriving and every section is compiled; returns the loaded chunks */
    private static int waitForChunks(ClientGameTestContext context) {
        int expected = (int) (Math.PI * RENDER_DISTANCE * RENDER_DISTANCE * 0.9);
        int[] last = {-1};
        int[] stableTicks = {0};
        context.waitFor(mc -> {
            int loaded = mc.level.getChunkSource().getLoadedChunksCount();
            boolean done = loaded == last[0] && loaded >= expected && mc.levelRenderer.hasRenderedAllSections();
            stableTicks[0] = done ? stableTicks[0] + 1 : 0;
            last[0] = loaded;
            return stableTicks[0] >= TICKS_PER_SECOND;
        }, 120 * TICKS_PER_SECOND);
        return last[0];
    }

    private static JsonObject snapshot(ClientGameTestContext context, String name, Path histoDir, String prefix) {
        // Committed heap as the game left it, before the GCs shrink anything
        long committedBefore = ManagementFactory.getMemoryMXBean().getHeapMemoryUsage().getCommitted();
        // Two full GCs (the histogram's own is one) so soft and finalizable leftovers are gone too
        System.gc();
        String histogram = diagnosticCommand("gcClassHistogram");
        if (name.equals(HEAP_DUMP)) {
            Path dump = histoDir.resolve(prefix + "-" + name + ".hprof").toAbsolutePath();
            try {
                Files.deleteIfExists(dump);
                ManagementFactory.getPlatformMXBean(com.sun.management.HotSpotDiagnosticMXBean.class).dumpHeap(dump.toString(), true);
            } catch (IOException e) {
                throw new RuntimeException("Heap dump failed", e);
            }
            LOGGER.info("Heap dump of snapshot {}: {}", name, dump);
        }
        JsonObject s = new JsonObject();
        s.addProperty("committedHeapBeforeGc", committedBefore);
        var heap = ManagementFactory.getMemoryMXBean().getHeapMemoryUsage();
        s.addProperty("liveHeap", heap.getUsed());
        s.addProperty("committedHeapAfterGc", heap.getCommitted());
        s.addProperty("histogramBytes", histogramTotal(histogram));
        long metaspace = 0, classSpace = 0, codeCache = 0, nonHeap = 0;
        for (MemoryPoolMXBean pool : ManagementFactory.getMemoryPoolMXBeans()) {
            if (pool.getType() != MemoryType.NON_HEAP) {
                continue;
            }
            long used = pool.getUsage().getUsed();
            nonHeap += used;
            if (pool.getName().equals("Metaspace")) {
                metaspace = used;
            } else if (pool.getName().equals("Compressed Class Space")) {
                classSpace = used;
            } else if (pool.getName().startsWith("CodeHeap")) {
                codeCache += used;
            }
        }
        s.addProperty("metaspace", metaspace);
        s.addProperty("compressedClassSpace", classSpace);
        s.addProperty("codeCache", codeCache);
        s.addProperty("nonHeap", nonHeap);
        for (BufferPoolMXBean pool : ManagementFactory.getPlatformMXBeans(BufferPoolMXBean.class)) {
            if (pool.getName().equals("direct")) {
                s.addProperty("directBuffers", pool.getMemoryUsed());
            }
        }
        s.addProperty("loadedClasses", ManagementFactory.getClassLoadingMXBean().getLoadedClassCount());
        long[] process = processMemory();
        s.addProperty("workingSet", process[0]);
        s.addProperty("privateBytes", process[1]);
        long[] gpu = gpuMemory();
        s.addProperty("gpuDedicated", gpu[0]);
        s.addProperty("gpuShared", gpu[1]);

        try {
            Files.writeString(histoDir.resolve(prefix + "-histo-" + name + ".txt"), firstLines(histogram, HISTOGRAM_ROWS + 3)
                    + histogram.lines().filter(l -> l.trim().startsWith("Total")).findFirst().orElse("") + "\n");
            if (ManagementFactory.getRuntimeMXBean().getInputArguments().stream().anyMatch(a -> a.startsWith("-XX:NativeMemoryTracking="))) {
                Files.writeString(histoDir.resolve(prefix + "-nmt-" + name + ".txt"), diagnosticCommand("vmNativeMemory", "summary"));
            }
        } catch (IOException e) {
            throw new RuntimeException(e);
        }
        LOGGER.info("Snapshot {}: live heap {} MB, private {} MB, GPU dedicated {} MB", name,
                "%.1f".formatted(heap.getUsed() / MB), "%.1f".formatted(process[1] / MB), "%.1f".formatted(gpu[0] / MB));
        return s;
    }

    private static String diagnosticCommand(String operation, String... args) {
        try {
            return (String) ManagementFactory.getPlatformMBeanServer().invoke(
                    new ObjectName("com.sun.management:type=DiagnosticCommand"), operation,
                    new Object[]{args}, new String[]{String[].class.getName()});
        } catch (Exception e) {
            throw new RuntimeException("Diagnostic command " + operation + " failed", e);
        }
    }

    /** The histogram's "Total" row: live bytes of all instances */
    private static long histogramTotal(String histogram) {
        for (String line : histogram.split("\n")) {
            String trimmed = line.trim();
            if (trimmed.startsWith("Total")) {
                String[] parts = trimmed.split("\\s+");
                return Long.parseLong(parts[2]);
            }
        }
        return -1;
    }

    /** Working set and private bytes (PROCESS_MEMORY_COUNTERS_EX) of this process; -1 off Windows */
    private static long[] processMemory() {
        if (!System.getProperty("os.name").toLowerCase().contains("windows")) {
            return new long[]{-1, -1};
        }
        try (Arena arena = Arena.ofConfined()) {
            Linker linker = Linker.nativeLinker();
            SymbolLookup kernel32 = SymbolLookup.libraryLookup("kernel32", arena);
            MethodHandle currentProcess = linker.downcallHandle(kernel32.find("GetCurrentProcess").orElseThrow(),
                    FunctionDescriptor.of(ValueLayout.ADDRESS));
            MethodHandle memoryInfo = linker.downcallHandle(kernel32.find("K32GetProcessMemoryInfo").orElseThrow(),
                    FunctionDescriptor.of(ValueLayout.JAVA_INT, ValueLayout.ADDRESS, ValueLayout.ADDRESS, ValueLayout.JAVA_INT));
            int size = 80;
            MemorySegment counters = arena.allocate(size, 8);
            counters.set(ValueLayout.JAVA_INT, 0, size);
            int ok = (int) memoryInfo.invokeExact((MemorySegment) currentProcess.invokeExact(), counters, size);
            if (ok == 0) {
                return new long[]{-1, -1};
            }
            return new long[]{counters.get(ValueLayout.JAVA_LONG, 16), counters.get(ValueLayout.JAVA_LONG, 72)};
        } catch (Throwable t) {
            LOGGER.warn("Could not read the process memory counters", t);
            return new long[]{-1, -1};
        }
    }

    /** Dedicated and shared GPU memory of this process (all adapters) from Windows' "GPU Process Memory" counters */
    private static long[] gpuMemory() {
        if (!System.getProperty("os.name").toLowerCase().contains("windows")) {
            return new long[]{-1, -1};
        }
        long pid = ProcessHandle.current().pid();
        try {
            Process process = new ProcessBuilder("typeperf",
                    "\\GPU Process Memory(pid_" + pid + "_*)\\Dedicated Usage",
                    "\\GPU Process Memory(pid_" + pid + "_*)\\Shared Usage", "-sc", "1")
                    .redirectErrorStream(true).start();
            String output = new String(process.getInputStream().readAllBytes(), StandardCharsets.UTF_8);
            process.waitFor();
            String[] lines = output.lines().filter(l -> l.startsWith("\"")).toArray(String[]::new);
            if (lines.length < 2) {
                LOGGER.warn("No GPU memory counters for pid {}: {}", pid, output.trim());
                return new long[]{-1, -1};
            }
            String[] headers = lines[0].split("\",\"");
            String[] values = lines[1].split("\",\"");
            long dedicated = 0, shared = 0;
            for (int i = 1; i < headers.length && i < values.length; i++) {
                long value = (long) Double.parseDouble(values[i].replace("\"", "").trim());
                if (headers[i].contains("Dedicated Usage")) {
                    dedicated += value;
                } else if (headers[i].contains("Shared Usage")) {
                    shared += value;
                }
            }
            return new long[]{dedicated, shared};
        } catch (IOException | InterruptedException | NumberFormatException e) {
            LOGGER.warn("Could not read the GPU memory counters", e);
            return new long[]{-1, -1};
        }
    }

    private static String firstLines(String text, int lines) {
        return text.lines().limit(lines).reduce(new StringBuilder(), (b, l) -> b.append(l).append('\n'), StringBuilder::append).toString();
    }

    private static JsonArray array(double[] values) {
        JsonArray array = new JsonArray();
        for (double value : values) {
            array.add(value);
        }
        return array;
    }

    private static double median(double[] values) {
        if (values.length == 0) {
            return Double.NaN;
        }
        double[] sorted = values.clone();
        Arrays.sort(sorted);
        return sorted[sorted.length / 2];
    }

    private static double mb(JsonObject s, String key) {
        return s.has(key) ? s.get(key).getAsLong() / MB : -1;
    }
}
