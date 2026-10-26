package net.vulkanmod.gametest;

import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.google.gson.JsonObject;
import com.mojang.blaze3d.systems.RenderSystem;
import com.mojang.brigadier.ParseResults;
import com.mojang.renderpearl.api.device.DeviceInfo;
import net.fabricmc.fabric.api.client.gametest.v1.FabricClientGameTest;
import net.fabricmc.fabric.api.client.gametest.v1.context.ClientGameTestContext;
import net.fabricmc.loader.api.FabricLoader;
import net.minecraft.client.GraphicsPreset;
import net.minecraft.client.Minecraft;
import net.minecraft.client.Options;
import net.minecraft.client.gui.screens.ConnectScreen;
import net.minecraft.client.gui.screens.TitleScreen;
import net.minecraft.client.multiplayer.ClientPacketListener;
import net.minecraft.client.multiplayer.ClientSuggestionProvider;
import net.minecraft.client.multiplayer.ServerData;
import net.minecraft.client.multiplayer.resolver.ServerAddress;
import net.minecraft.client.particle.ParticleGroup;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.EntityType;
import net.vulkanmod.gametest.mixin.ParticleEngineAccessor;
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
 * Multiplayer FPS benchmark.
 * <ol>
 *     <li>Join the benchmark server and teleport to the benchmark spot.</li>
 *     <li>"initial": the first {@code bench.initialSeconds} after the teleport, while FPS settles.</li>
 *     <li>"pan": a full 360 degree yaw pan over {@code bench.panSeconds}.</li>
 *     <li>"static": {@code bench.staticSeconds} looking at the particle spot, as a reference for the particle phase.</li>
 *     <li>"particles": {@code bench.particleSeconds} after spawning 500k happy_villager particles in view.</li>
 * </ol>
 * All settings can be overridden with -Pbench.&lt;name&gt;=value on the Gradle bench* tasks.
 */
public class BenchmarkClientGameTest implements FabricClientGameTest {
    private static final Logger LOGGER = LoggerFactory.getLogger("VulkanMod Benchmark");

    private static final String LABEL = prop("label", "dev");
    private static final String SERVER = prop("server", "62.141.62.9:10002");
    private static final String PLAYER = prop("player", "6a0");
    private static final double TP_X = -98.5, TP_Y = 69, TP_Z = 189.5;
    private static final String TP_COMMAND = "tp %s %s %s %s 0 0".formatted(PLAYER, TP_X, TP_Y, TP_Z);
    private static final String PARTICLE_COMMAND = prop("particleCommand",
            "particle minecraft:happy_villager -97.5 68 199 2 2 2 1 500000");

    private static final double INITIAL_SECONDS = Double.parseDouble(prop("initialSeconds", "5"));
    private static final double PAN_SECONDS = Double.parseDouble(prop("panSeconds", "10"));
    private static final double STATIC_SECONDS = Double.parseDouble(prop("staticSeconds", "5"));
    private static final double PARTICLE_SECONDS = Double.parseDouble(prop("particleSeconds", "10"));
    private static final String PRESET = prop("preset", "fancy");
    private static final int RENDER_DISTANCE = Integer.parseInt(prop("renderDistance", "16"));
    private static final String[] EXTRA_SHOT_YAWS = prop("extraShotYaws", "270").split(",");

    private static final int TICKS_PER_SECOND = 20;
    private static final String RUN_ID = LocalDateTime.now().format(DateTimeFormatter.ofPattern("yyyyMMdd-HHmmss"));

    @Override
    public void runTest(ClientGameTestContext context) {
        context.runOnClient(_ -> DisplayKeepAwake.enable());
        context.runOnClient(this::applyOptions);

        connect(context);
        teleport(context);

        List<PhaseResult> results = new ArrayList<>();
        Map<String, String> frameSeries = new HashMap<>();

        // Initial phase: measure while FPS settles after the teleport
        context.runOnClient(_ -> FrameRecorder.startPhase());
        waitSeconds(context, INITIAL_SECONDS);
        results.add(context.computeOnClient(_ -> FrameRecorder.stopPhase("initial", -1)));
        LOGGER.info(results.getLast().summaryLine());

        // 360 pan
        context.runOnClient(mc -> {
            FrameRecorder.startPhase();
            FrameRecorder.startPan(mc, 0.0f, PAN_SECONDS);
        });
        context.waitFor(_ -> !FrameRecorder.isPanning(), (int) (PAN_SECONDS * TICKS_PER_SECOND * 10));
        results.add(context.computeOnClient(_ -> FrameRecorder.stopPhase("pan", -1)));
        frameSeries.put("pan", context.computeOnClient(_ -> FrameRecorder.framesCsv()));
        LOGGER.info(results.getLast().summaryLine());

        // Static view at the particle spot, as the reference for the particle phase.
        // The screenshot (taken before measuring) is for comparing rendering output between configurations.
        for (String yaw : EXTRA_SHOT_YAWS) {
            if (!yaw.isBlank()) {
                takeScreenshotAtYaw(context, Float.parseFloat(yaw.trim()));
            }
        }
        setYaw(context, 0.0f);

        context.takeScreenshot(LABEL + "-static-" + RUN_ID);
        context.runOnClient(_ -> FrameRecorder.startPhase());
        waitSeconds(context, STATIC_SECONDS);
        results.add(context.computeOnClient(_ -> FrameRecorder.stopPhase("static", -1)));
        LOGGER.info(results.getLast().summaryLine());

        // Particle stress test
        context.runOnClient(mc -> {
            FrameRecorder.startPhase();
            mc.player.connection.sendCommand(PARTICLE_COMMAND);
        });
        int peakParticles = 0;
        long end = System.nanoTime() + (long) (PARTICLE_SECONDS * 1.0e9);
        while (System.nanoTime() < end) {
            context.waitTicks(5);
            peakParticles = Math.max(peakParticles, context.computeOnClient(BenchmarkClientGameTest::countParticles));
        }
        if (peakParticles < 1000) {
            throw new AssertionError("Particle command had no effect (peak " + peakParticles + " particles)");
        }
        int finalPeak = peakParticles;
        results.add(context.computeOnClient(_ -> FrameRecorder.stopPhase("particles", finalPeak)));
        LOGGER.info(results.getLast().summaryLine());

        JsonObject environment = context.computeOnClient(this::describeEnvironment);
        writeResults(results, environment, frameSeries);

        context.runOnClient(mc -> mc.disconnect(new TitleScreen(), false));
        context.waitForScreen(TitleScreen.class);
    }

    private void applyOptions(Minecraft mc) {
        Options options = mc.options;
        options.applyGraphicsPreset(GraphicsPreset.valueOf(PRESET.toUpperCase(Locale.ROOT)));
        options.graphicsPreset().set(GraphicsPreset.valueOf(PRESET.toUpperCase(Locale.ROOT)));
        options.renderDistance().set(RENDER_DISTANCE);
        options.framerateLimit().set(Options.UNLIMITED_FRAMERATE_CUTOFF);
        options.enableVsync().set(false);
        options.pauseOnLostFocus = false;
    }

    private void connect(ClientGameTestContext context) {
        LOGGER.info("Connecting to {} as {}", SERVER, PLAYER);
        context.runOnClient(mc -> ConnectScreen.startConnecting(new TitleScreen(), mc, ServerAddress.parseString(SERVER),
                new ServerData("Benchmark", SERVER, ServerData.Type.OTHER), false, null));

        context.waitFor(mc -> mc.player != null && mc.level != null && mc.gui.screen() == null,
                60 * TICKS_PER_SECOND);
    }

    /**
     * Fails fast if the server would reject the command (the server only sends the command tree the player is
     * permitted to use, so an unpermitted command fails to parse client side).
     */
    private static void requireCommand(ClientGameTestContext context, String command) {
        String error = context.computeOnClient(mc -> {
            ClientPacketListener connection = mc.player.connection;
            ParseResults<ClientSuggestionProvider> parse = connection.getCommands().parse(command, connection.getSuggestionsProvider());
            if (!parse.getExceptions().isEmpty()) {
                return parse.getExceptions().values().iterator().next().getMessage();
            }
            if (parse.getReader().canRead()) {
                return "unknown or incomplete command";
            }
            return null;
        });

        if (error != null) {
            throw new AssertionError("Server rejects '/" + command + "' (" + error + "); "
                    + PLAYER + " needs operator permissions on the benchmark server");
        }
    }

    private void teleport(ClientGameTestContext context) {
        requireCommand(context, TP_COMMAND);
        requireCommand(context, PARTICLE_COMMAND);

        context.runOnClient(mc -> mc.player.connection.sendCommand(TP_COMMAND));

        try {
            context.waitFor(mc -> {
                LocalPlayer player = mc.player;
                return player != null && player.distanceToSqr(TP_X, TP_Y, TP_Z) < 0.01;
            }, 10 * TICKS_PER_SECOND);
        } catch (AssertionError e) {
            throw new AssertionError("Teleport did not happen; does " + PLAYER + " have permission to use /tp?", e);
        }
    }

    /**
     * Screenshots of other view directions, taken between the pan and the static phase, for checking rendering
     * parity where the static view doesn't look.
     */
    private static void takeScreenshotAtYaw(ClientGameTestContext context, float yaw) {
        setYaw(context, yaw);
        context.takeScreenshot(LABEL + "-yaw" + (int) yaw + "-" + RUN_ID);
    }

    private static void setYaw(ClientGameTestContext context, float yaw) {
        context.runOnClient(mc -> {
            mc.player.setYRot(yaw);
            mc.player.yRotO = yaw;
        });
        // Let section visibility and compilation catch up with the new direction
        context.waitTicks(20);
    }

    private static void waitSeconds(ClientGameTestContext context, double seconds) {
        long end = System.nanoTime() + (long) (seconds * 1.0e9);
        while (System.nanoTime() < end) {
            context.waitTick();
        }
    }

    private static int countParticles(Minecraft mc) {
        int total = 0;
        for (ParticleGroup<?> group : ((ParticleEngineAccessor) mc.particleEngine).getParticles().values()) {
            total += group.size();
        }
        return total;
    }

    private JsonObject describeEnvironment(Minecraft mc) {
        DeviceInfo device = RenderSystem.getDevice().getDeviceInfo();
        JsonObject env = new JsonObject();
        env.addProperty("label", LABEL);
        env.addProperty("backend", device.backendName());
        env.addProperty("gpu", device.name());
        env.addProperty("driver", device.driverInfo());
        env.addProperty("vulkanmod", FabricLoader.getInstance().isModLoaded("vulkanmod"));
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

    private static void writeResults(List<PhaseResult> results, JsonObject environment, Map<String, String> frameSeries) {
        Gson gson = new GsonBuilder().setPrettyPrinting().create();
        JsonObject root = new JsonObject();
        root.add("environment", environment);
        root.add("phases", gson.toJsonTree(results));

        StringBuilder summary = new StringBuilder();
        summary.append("== Benchmark: ").append(LABEL).append(" ==\n").append(gson.toJson(environment)).append('\n');
        for (PhaseResult result : results) {
            summary.append(result.summaryLine()).append('\n');
        }
        LOGGER.info("\n{}", summary);

        String timestamp = LocalDateTime.now().format(DateTimeFormatter.ofPattern("yyyyMMdd-HHmmss"));
        Path outDir = Path.of(prop("outDir", "../results"));
        try {
            Files.createDirectories(outDir);
            Files.writeString(outDir.resolve(LABEL + "-" + timestamp + ".json"), gson.toJson(root));
            Files.writeString(outDir.resolve(LABEL + "-" + timestamp + ".txt"), summary);
            Path framesDir = outDir.resolve("frames");
            Files.createDirectories(framesDir);
            for (Map.Entry<String, String> series : frameSeries.entrySet()) {
                Files.writeString(framesDir.resolve(LABEL + "-" + timestamp + "-" + series.getKey() + ".csv"), series.getValue());
            }
        } catch (IOException e) {
            throw new RuntimeException("Failed to write benchmark results", e);
        }
    }

    private static String prop(String name, String def) {
        return System.getProperty("vulkanmod.bench." + name, def);
    }
}
