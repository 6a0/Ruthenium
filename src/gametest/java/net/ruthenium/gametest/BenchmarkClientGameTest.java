package net.ruthenium.gametest;

import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.google.gson.JsonObject;
import com.mojang.blaze3d.systems.RenderSystem;
import com.mojang.renderpearl.api.device.DeviceInfo;
import net.fabricmc.fabric.api.client.gametest.v1.FabricClientGameTest;
import net.fabricmc.fabric.api.client.gametest.v1.context.ClientGameTestContext;
import net.fabricmc.loader.api.FabricLoader;
import net.minecraft.client.GraphicsPreset;
import net.minecraft.client.InactivityFpsLimit;
import net.minecraft.client.Minecraft;
import net.minecraft.client.Options;
import net.minecraft.client.particle.ParticleGroup;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.EntityType;
import net.ruthenium.gametest.mixin.ParticleEngineAccessor;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.time.LocalDateTime;
import java.time.format.DateTimeFormatter;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;

/**
 * The client benchmark: sets the options every scene shares (graphics preset, render distance, unlimited fps, no
 * vsync), then runs one singleplayer scene, chosen with {@code bench.scene}: "mixed" (the default,
 * {@link MixedBenchmark}), "terrain" ({@link TerrainBenchmark}), "blockentities" ({@link BlockEntityBenchmark}) or
 * "entities" ({@link EntityBenchmark}). All settings can be overridden with -Pbench.&lt;name&gt;=value on the Gradle
 * bench* tasks.
 */
public class BenchmarkClientGameTest implements FabricClientGameTest {
    private static final Logger LOGGER = LoggerFactory.getLogger("Ruthenium Benchmark");

    private static final String LABEL = prop("label", "dev");
    static final double PAN_SECONDS = Double.parseDouble(prop("panSeconds", "10"));
    static final double STATIC_SECONDS = Double.parseDouble(prop("staticSeconds", "5"));
    private static final String PRESET = prop("preset", "fancy");
    private static final int RENDER_DISTANCE = Integer.parseInt(prop("renderDistance", "16"));
    /** Yaw of the static phase (and the particle phase); only for profiling other view directions */
    static final float STATIC_YAW = Float.parseFloat(prop("staticYaw", "0"));
    /** Instead of the benchmark, drives the settings screen from the title screen (see {@link SettingsScreenCheck}) */
    private static final boolean SETTINGS_SCREEN = Boolean.parseBoolean(prop("settingsScreen", "false"));
    private static final String SCENE = prop("scene", "mixed");

    private static final String RUN_ID = LocalDateTime.now().format(DateTimeFormatter.ofPattern("yyyyMMdd-HHmmss"));

    @Override
    public void runTest(ClientGameTestContext context) {
        context.runOnClient(_ -> DisplayKeepAwake.enable());
        if (SETTINGS_SCREEN) {
            SettingsScreenCheck.run(context, LABEL);
            return;
        }
        context.runOnClient(BenchmarkClientGameTest::applyOptions);
        // Diagnostic: extra swapchain reconfigurations after the options (bench.extraReconfigures, a second apart)
        int extraReconfigures = Integer.parseInt(prop("extraReconfigures", "0"));
        for (int i = 0; i < extraReconfigures; i++) {
            context.waitTicks(20);
            context.runOnClient(Minecraft::invalidateSurfaceConfiguration);
        }
        switch (SCENE) {
            case "mixed" -> MixedBenchmark.run(context, LABEL, RUN_ID);
            case "terrain" -> TerrainBenchmark.run(context, LABEL, RUN_ID);
            case "blockentities" -> BlockEntityBenchmark.run(context, LABEL, RUN_ID);
            case "entities" -> EntityBenchmark.run(context, LABEL, RUN_ID);
            case "players" -> PlayerBenchmark.run(context, LABEL, RUN_ID);
            default -> throw new AssertionError("Unknown scene '" + SCENE + "' (mixed, terrain, blockentities, entities or players)");
        }
    }

    private static void applyOptions(Minecraft mc) {
        Options options = mc.options;
        options.applyGraphicsPreset(GraphicsPreset.valueOf(PRESET.toUpperCase(Locale.ROOT)));
        options.graphicsPreset().set(GraphicsPreset.valueOf(PRESET.toUpperCase(Locale.ROOT)));
        options.renderDistance().set(RENDER_DISTANCE);
        options.framerateLimit().set(Options.UNLIMITED_FRAMERATE_CUTOFF);
        options.enableVsync().set(false);
        // The AFK limit caps the frame rate at 30 fps after 60 s without input (e.g. while a world generates)
        options.inactivityFpsLimit().set(InactivityFpsLimit.MINIMIZED);
        options.pauseOnLostFocus = false;
    }

    /** Turns the player and lets section visibility and compilation catch up with the new direction */
    static void setYaw(ClientGameTestContext context, float yaw) {
        context.runOnClient(mc -> {
            mc.player.setYRot(yaw);
            mc.player.yRotO = yaw;
        });
        context.waitTicks(20);
    }

    static void waitSeconds(ClientGameTestContext context, double seconds) {
        long end = System.nanoTime() + (long) (seconds * 1.0e9);
        while (System.nanoTime() < end) {
            context.waitTick();
        }
    }

    static int countParticles(Minecraft mc) {
        int total = 0;
        for (ParticleGroup<?> group : ((ParticleEngineAccessor) mc.particleEngine).getParticles().values()) {
            total += group.size();
        }
        return total;
    }

    static JsonObject describeEnvironment(Minecraft mc) {
        DeviceInfo device = RenderSystem.getDevice().getDeviceInfo();
        JsonObject env = new JsonObject();
        env.addProperty("label", LABEL);
        env.addProperty("backend", device.backendName());
        env.addProperty("gpu", device.name());
        env.addProperty("driver", device.driverInfo());
        env.addProperty("ruthenium", FabricLoader.getInstance().isModLoaded("ruthenium"));
        env.addProperty("sodium", FabricLoader.getInstance().isModLoaded("sodium"));
        env.addProperty("window", mc.getWindow().getWidth() + "x" + mc.getWindow().getHeight());
        env.addProperty("preset", PRESET);
        env.addProperty("renderDistance", mc.options.getEffectiveRenderDistance());
        env.addProperty("java", System.getProperty("java.vm.name") + " " + System.getProperty("java.version"));
        env.addProperty("maxHeapMB", Runtime.getRuntime().maxMemory() / (1024 * 1024));
        env.addProperty("cpus", Runtime.getRuntime().availableProcessors());

        Map<String, Integer> entityCounts = new HashMap<>();
        for (Entity entity : mc.level.entitiesForRendering()) {
            entityCounts.merge(EntityType.getKey(entity.getType()).toString(), 1, Integer::sum);
        }
        JsonObject entities = new JsonObject();
        entityCounts.entrySet().stream()
                .sorted(Map.Entry.<String, Integer>comparingByValue().reversed())
                .forEach(e -> entities.addProperty(e.getKey(), e.getValue()));
        env.addProperty("entityCount", mc.level.getEntityCount());
        env.add("entities", entities);
        return env;
    }

    static void writeResults(String label, List<PhaseResult> results, JsonObject environment, Map<String, String> frameSeries) {
        Gson gson = new GsonBuilder().setPrettyPrinting().create();
        JsonObject root = new JsonObject();
        root.add("environment", environment);
        root.add("phases", gson.toJsonTree(results));

        StringBuilder summary = new StringBuilder();
        summary.append("== Benchmark: ").append(label).append(" ==\n").append(gson.toJson(environment)).append('\n');
        for (PhaseResult result : results) {
            summary.append(result.summaryLine()).append('\n');
        }
        LOGGER.info("\n{}", summary);

        String timestamp = LocalDateTime.now().format(DateTimeFormatter.ofPattern("yyyyMMdd-HHmmss"));
        Path outDir = Path.of(prop("outDir", "../results"));
        try {
            Files.createDirectories(outDir);
            Files.writeString(outDir.resolve(label + "-" + timestamp + ".json"), gson.toJson(root));
            Files.writeString(outDir.resolve(label + "-" + timestamp + ".txt"), summary);
            Path framesDir = outDir.resolve("frames");
            Files.createDirectories(framesDir);
            for (Map.Entry<String, String> series : frameSeries.entrySet()) {
                Files.writeString(framesDir.resolve(label + "-" + timestamp + "-" + series.getKey() + ".csv"), series.getValue());
            }
        } catch (IOException e) {
            throw new RuntimeException("Failed to write benchmark results", e);
        }
    }

    static String prop(String name, String def) {
        return System.getProperty("ruthenium.bench." + name, def);
    }
}
