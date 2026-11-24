package dev.ruthenium.bench;

import com.google.gson.GsonBuilder;
import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import com.mojang.blaze3d.systems.RenderSystem;
import java.io.IOException;
import java.lang.management.GarbageCollectorMXBean;
import java.lang.management.ManagementFactory;
import java.lang.management.MemoryUsage;
import java.nio.file.Files;
import java.nio.file.Path;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.screens.DisconnectedScreen;
import net.minecraft.client.gui.screens.TitleScreen;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.core.BlockPos;
import net.minecraft.network.chat.Component;
import net.minecraft.server.MinecraftServer;
import net.minecraft.world.level.LevelSettings;
import net.minecraft.world.level.WorldDataConfiguration;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.gamerules.GameRules;
import net.minecraft.world.level.levelgen.WorldOptions;
import net.minecraft.world.level.levelgen.presets.WorldPresets;

/**
 * Drives one benchmark scene from the frame-end hook (render thread) and writes what it measured.
 *
 * <p>Scenes: {@code menu} (title screen; also gives launch times), {@code idle} (stand still),
 * {@code spin} (turn on the spot, 36 degrees per second), {@code fly} (sprint-fly south through
 * pre-generated terrain), and {@code setup} (create the benchmark world and pre-generate the fly
 * path). World scenes are started with {@code --quickPlaySingleplayer} on a fresh copy of the world
 * made by {@code setup}.
 *
 * <p>With {@code ruthenium.bench.route} (cargo xtask bench dedicated), the scene is a {@link Route}
 * played on a dedicated server the client joined with {@code --quickPlayMultiplayer}: a controller
 * on the server sets the scene up, {@link RouteRun} drives the player by client tick, and frame
 * intervals are measured over the route's capture ticks instead of over a time.
 *
 * <p>Configuration comes from system properties set by cargo xtask bench: {@code
 * ruthenium.bench.scene}, {@code .out}, {@code .warmup}, {@code .duration} (seconds), {@code .world},
 * {@code .seed} and {@code .route} (a route file).
 */
public final class Bench {
    private static final long SECOND = 1_000_000_000L;
    private static final double SPIN_DEGREES_PER_SECOND = 36.0;
    /** {@code Entity.turn} multiplies mouse movement by this to get degrees. */
    private static final double TURN_SCALE = 0.15;
    private static final int FLY_HEIGHT = 40;
    private static final int PREGEN_STEP = 48;
    /** Sprint-flying covers about 22 blocks per second; pre-generate a margin beyond that. */
    private static final double PREGEN_BLOCKS_PER_SECOND = 25.0;

    private static Bench active;

    private enum Phase { STARTING, LOADING_WORLD, SETTLING, WARMUP, MEASURING, ROUTE, PREGENERATING, SAVING, DONE }

    private final String scene;
    private final Path out;
    private final long warmupNs;
    private final long durationNs;
    private final String worldName;
    private final long seed;
    /** The route to play on a dedicated server, or null for the integrated-server scenes. */
    private final RouteRun routeRun;
    private boolean capturing;
    private final long jvmStartEpochMs = ManagementFactory.getRuntimeMXBean().getStartTime();

    private Phase phase = Phase.STARTING;
    private long phaseStartNs = System.nanoTime();
    private long stableSinceNs = -1;
    private long firstFrameEpochMs = -1;
    private long menuReadyEpochMs = -1;
    private long worldReadyEpochMs = -1;
    private long measureStartEpochMs;
    private long measureEndEpochMs;
    private long lastFrameNs;
    private long[] intervals = new long[1 << 16];
    private long[] renderCpu = new long[1 << 16];
    private int frames;
    private long gcCountStart;
    private long gcTimeStart;
    private BlockPos spawn;
    private int pregenStep;
    private int pregenSteps;
    private double[] playerStart;

    private Bench(String scene) {
        this.scene = scene;
        this.out = Path.of(System.getProperty("ruthenium.bench.out", "ruthenium-bench.json"));
        this.warmupNs = (long) (Double.parseDouble(System.getProperty("ruthenium.bench.warmup", "10")) * SECOND);
        this.durationNs = (long) (Double.parseDouble(System.getProperty("ruthenium.bench.duration", "60")) * SECOND);
        this.worldName = System.getProperty("ruthenium.bench.world", "ruthenium-bench");
        this.seed = Long.parseLong(System.getProperty("ruthenium.bench.seed", "0"));
        String route = System.getProperty("ruthenium.bench.route");
        this.routeRun = route == null ? null : new RouteRun(Route.load(Path.of(route)));
    }

    static void init() {
        String scene = System.getProperty("ruthenium.bench.scene");
        if (scene != null) {
            active = new Bench(scene);
        }
    }

    /** True when cargo xtask bench started this client: the player's input is then ignored. */
    public static boolean isActive() {
        return active != null;
    }

    /** World loading and saving show screens, which render frames from inside our own call. */
    private static boolean inFrame;

    public static void onFrameEnd(Minecraft mc) {
        if (active != null && !inFrame) {
            inFrame = true;
            try {
                active.frame(mc, System.nanoTime());
            } finally {
                inFrame = false;
            }
        }
    }

    /** The end of a client tick: a route is keyed by these. */
    public static void onTickEnd(Minecraft mc) {
        if (active != null && active.routeRun != null && active.phase == Phase.ROUTE) {
            active.routeRun.tick(mc);
        }
    }

    private void enter(Phase next, long now) {
        phase = next;
        phaseStartNs = now;
        stableSinceNs = -1;
    }

    private boolean isWorld() {
        return !scene.equals("menu");
    }

    private void frame(Minecraft mc, long now) {
        if (firstFrameEpochMs < 0) {
            firstFrameEpochMs = System.currentTimeMillis();
        }
        switch (phase) {
            case STARTING -> {
                boolean menu = mc.gui.overlay() == null && mc.gui.screen() instanceof TitleScreen;
                if (menu && menuReadyEpochMs < 0) {
                    menuReadyEpochMs = System.currentTimeMillis();
                }
                if (scene.equals("menu") && menu) {
                    enter(Phase.WARMUP, now);
                } else if (scene.equals("setup") && menu) {
                    enter(Phase.LOADING_WORLD, now);
                    createWorld(mc);
                } else if (isWorld() && !scene.equals("setup") && mc.level != null) {
                    enter(Phase.LOADING_WORLD, now);
                } else if (routeRun != null && mc.gui.screen() instanceof DisconnectedScreen) {
                    routeRun.fail("the client could not join the server");
                    write(mc);
                    enter(Phase.DONE, now);
                    mc.stop();
                }
            }
            case LOADING_WORLD -> {
                if (mc.level != null && mc.player != null && mc.gui.screen() == null && mc.gui.overlay() == null) {
                    worldReadyEpochMs = System.currentTimeMillis();
                    if (routeRun != null) {
                        enter(Phase.ROUTE, now);
                    } else {
                        startWorldScene(mc);
                        enter(Phase.SETTLING, now);
                    }
                }
            }
            case ROUTE -> route(mc, now);
            case SETTLING -> {
                keepFlying(mc);
                if (settled(mc, now, 3 * SECOND, 5 * SECOND, 120 * SECOND)) {
                    if (scene.equals("setup")) {
                        pregenSteps = (int) Math.ceil(
                                PREGEN_BLOCKS_PER_SECOND * (warmupNs + durationNs + 20 * SECOND) / SECOND / PREGEN_STEP);
                        enter(Phase.PREGENERATING, now);
                    } else {
                        playerStart = position(mc.player);
                        beginMotion(mc);
                        enter(Phase.WARMUP, now);
                    }
                }
            }
            case WARMUP -> {
                applyMotion(mc, now);
                if (now - phaseStartNs >= warmupNs) {
                    measureStartEpochMs = System.currentTimeMillis();
                    long[] gc = gc();
                    gcCountStart = gc[0];
                    gcTimeStart = gc[1];
                    lastFrameNs = now;
                    enter(Phase.MEASURING, now);
                    if (Boolean.getBoolean("ruthenium.bench.screenshot")) {
                        net.minecraft.client.Screenshot.grab(
                                mc.gameDirectory, "ruthenium-bench-" + scene + ".png", mc.gameRenderer.mainRenderTarget(), 1, msg -> {});
                    }
                }
            }
            case MEASURING -> {
                applyMotion(mc, now);
                if (frames == intervals.length) {
                    intervals = java.util.Arrays.copyOf(intervals, frames * 2);
                    renderCpu = java.util.Arrays.copyOf(renderCpu, frames * 2);
                }
                intervals[frames] = now - lastFrameNs;
                renderCpu[frames] = mc.getFrameTimeNs();
                frames++;
                lastFrameNs = now;
                if (now - phaseStartNs >= durationNs) {
                    measureEndEpochMs = System.currentTimeMillis();
                    releaseKeys(mc);
                    write(mc);
                    enter(Phase.DONE, now);
                    mc.stop();
                }
            }
            case PREGENERATING -> pregenerate(mc, now);
            case SAVING -> {
                if (mc.level == null && mc.gui.screen() instanceof TitleScreen) {
                    write(mc);
                    enter(Phase.DONE, now);
                    mc.stop();
                }
            }
            case DONE -> {
            }
        }
    }

    /**
     * A frame of a route: the route's ticks say when the capture starts and ends, and this records
     * the frame intervals in between.
     */
    private void route(Minecraft mc, long now) {
        if (mc.level == null || mc.player == null) {
            routeRun.fail("the connection to the server closed");
        }
        routeRun.setRendered(settled(mc, now, 3 * SECOND, 5 * SECOND, 120 * SECOND));
        if (capturing) {
            if (frames == intervals.length) {
                intervals = java.util.Arrays.copyOf(intervals, frames * 2);
                renderCpu = java.util.Arrays.copyOf(renderCpu, frames * 2);
            }
            intervals[frames] = now - lastFrameNs;
            renderCpu[frames] = mc.getFrameTimeNs();
            frames++;
            lastFrameNs = now;
            if (!routeRun.capturing()) {
                measureEndEpochMs = System.currentTimeMillis();
                capturing = false;
            }
        } else if (routeRun.capturing()) {
            measureStartEpochMs = System.currentTimeMillis();
            long[] gc = gc();
            gcCountStart = gc[0];
            gcTimeStart = gc[1];
            lastFrameNs = now;
            capturing = true;
            if (Boolean.getBoolean("ruthenium.bench.screenshot")) {
                net.minecraft.client.Screenshot.grab(
                        mc.gameDirectory, "ruthenium-bench-" + scene + ".png", mc.gameRenderer.mainRenderTarget(), 1, msg -> {});
            }
        }
        if (routeRun.finished()) {
            write(mc);
            enter(Phase.DONE, now);
            mc.stop();
        }
    }

    /** True once all queued section meshes have been built for {@code stable}, after at least {@code min}. */
    private boolean settled(Minecraft mc, long now, long stable, long min, long max) {
        if (!mc.levelRenderer.hasRenderedAllSections()) {
            stableSinceNs = -1;
        } else if (stableSinceNs < 0) {
            stableSinceNs = now;
        }
        long inPhase = now - phaseStartNs;
        return inPhase >= max || (inPhase >= min && stableSinceNs >= 0 && now - stableSinceNs >= stable);
    }

    private void createWorld(Minecraft mc) {
        LevelSettings settings = new LevelSettings(
                worldName, GameType.CREATIVE, LevelSettings.DifficultySettings.DEFAULT, true, WorldDataConfiguration.DEFAULT);
        mc.createWorldOpenFlows().createFreshLevel(
                worldName, settings, new WorldOptions(seed, true, false), WorldPresets::createNormalWorldDimensions, mc.gui.screen());
    }

    private static void command(Minecraft mc, String command) {
        MinecraftServer server = mc.getSingleplayerServer();
        if (server != null) {
            server.execute(() -> server.getCommands().performPrefixedCommand(server.createCommandSourceStack(), command));
        }
    }

    private void startWorldScene(Minecraft mc) {
        MinecraftServer server = mc.getSingleplayerServer();
        if (server != null) {
            server.execute(() -> {
                GameRules rules = server.getGameRules();
                rules.set(GameRules.ADVANCE_TIME, false, server);
                rules.set(GameRules.ADVANCE_WEATHER, false, server);
                rules.set(GameRules.SPAWN_MOBS, false, server);
            });
        }
        command(mc, "time set 6000");
        command(mc, "weather clear");
        LocalPlayer player = mc.player;
        if (scene.equals("setup")) {
            spawn = player.blockPosition();
            command(mc, tp(spawn.getX() + 0.5, spawn.getY(), spawn.getZ() + 0.5));
        } else if (scene.equals("fly")) {
            flyFromY = player.getY() + FLY_HEIGHT;
            command(mc, tp(player.getX(), flyFromY, player.getZ()));
        }
    }

    private static String tp(double x, double y, double z) {
        return "tp @a " + x + " " + y + " " + z + " 0 0";
    }

    private double flyFromY = Double.NaN;

    /**
     * Fly scene: once the teleport up has arrived, keep creative flight on. (Vanilla turns flight
     * off on landing, so it must only be switched on in the air.)
     */
    private void keepFlying(Minecraft mc) {
        LocalPlayer player = mc.player;
        if (scene.equals("fly") && player != null && player.getY() >= flyFromY - 1 && !player.getAbilities().flying) {
            player.getAbilities().flying = true;
            player.onUpdateAbilities();
        }
    }

    private void beginMotion(Minecraft mc) {
    }

    private void applyMotion(Minecraft mc, long now) {
        if (scene.equals("fly")) {
            keepFlying(mc);
            // Re-asserted every frame: vanilla resets key states to the physical keys when it opens a
            // screen.
            mc.options.keyUp.setDown(true);
            mc.options.keySprint.setDown(true);
        }
        if (scene.equals("spin") && mc.player != null) {
            double seconds = (now - lastMotionNs) / (double) SECOND;
            if (lastMotionNs != 0) {
                mc.player.turn(SPIN_DEGREES_PER_SECOND * seconds / TURN_SCALE, 0.0);
            }
        }
        lastMotionNs = now;
    }

    private long lastMotionNs;

    private static void releaseKeys(Minecraft mc) {
        mc.options.keyUp.setDown(false);
        mc.options.keySprint.setDown(false);
    }

    /** Teleports along the fly path step by step, waiting for each stretch to generate and mesh. */
    private void pregenerate(Minecraft mc, long now) {
        if (pregenStep > pregenSteps) {
            if (settled(mc, now, 2 * SECOND, 2 * SECOND, 60 * SECOND)) {
                enter(Phase.SAVING, now);
                mc.disconnectFromWorld(Component.literal("Benchmark world saved"));
            }
            return;
        }
        if (pregenStep == 0 || settled(mc, now, SECOND, SECOND, 30 * SECOND)) {
            pregenStep++;
            if (pregenStep > pregenSteps) {
                command(mc, tp(spawn.getX() + 0.5, spawn.getY(), spawn.getZ() + 0.5));
            } else {
                command(mc, tp(spawn.getX() + 0.5, spawn.getY() + FLY_HEIGHT, spawn.getZ() + 0.5 + pregenStep * PREGEN_STEP));
            }
            phaseStartNs = now;
            stableSinceNs = -1;
        }
    }

    private static long[] gc() {
        long count = 0;
        long time = 0;
        for (GarbageCollectorMXBean bean : ManagementFactory.getGarbageCollectorMXBeans()) {
            count += Math.max(0, bean.getCollectionCount());
            time += Math.max(0, bean.getCollectionTime());
        }
        return new long[] {count, time};
    }

    private static double[] position(LocalPlayer player) {
        return player == null ? null : new double[] {player.getX(), player.getY(), player.getZ()};
    }

    private static JsonArray array(double[] values) {
        JsonArray a = new JsonArray();
        if (values != null) {
            for (double v : values) {
                a.add(v);
            }
        }
        return a;
    }

    private void write(Minecraft mc) {
        JsonObject o = new JsonObject();
        o.addProperty("format", "ruthenium-bench/1");
        o.addProperty("scene", scene);
        o.addProperty("seed", seed);
        o.addProperty("jvmStartEpochMs", jvmStartEpochMs);
        o.addProperty("firstFrameEpochMs", firstFrameEpochMs);
        o.addProperty("menuReadyEpochMs", menuReadyEpochMs);
        o.addProperty("worldReadyEpochMs", worldReadyEpochMs);
        o.addProperty("measureStartEpochMs", measureStartEpochMs);
        o.addProperty("measureEndEpochMs", measureEndEpochMs);

        var info = RenderSystem.getDevice().getDeviceInfo();
        JsonObject device = new JsonObject();
        device.addProperty("name", info.name());
        device.addProperty("vendor", info.vendorName());
        device.addProperty("driver", info.driverInfo());
        device.addProperty("backend", info.backendName());
        o.add("device", device);
        o.addProperty("framebufferWidth", mc.getWindow().getWidth());
        o.addProperty("framebufferHeight", mc.getWindow().getHeight());
        o.addProperty("renderDistance", mc.options.getEffectiveRenderDistance());

        long[] gc = gc();
        o.addProperty("gcCount", gc[0] - gcCountStart);
        o.addProperty("gcTimeMs", gc[1] - gcTimeStart);
        MemoryUsage heap = ManagementFactory.getMemoryMXBean().getHeapMemoryUsage();
        o.addProperty("heapUsedBytes", heap.getUsed());
        o.addProperty("heapCommittedBytes", heap.getCommitted());
        o.add("playerStart", array(playerStart));
        o.add("playerEnd", array(position(mc.player)));
        if (spawn != null) {
            o.add("spawn", array(new double[] {spawn.getX(), spawn.getY(), spawn.getZ()}));
        }

        JsonArray iv = new JsonArray();
        JsonArray rc = new JsonArray();
        for (int i = 0; i < frames; i++) {
            iv.add(intervals[i]);
            rc.add(renderCpu[i]);
        }
        o.add("frameIntervalsNs", iv);
        o.add("renderCpuNs", rc);
        if (routeRun != null) {
            o.add("routeRun", routeRun.toJson());
        }
        try {
            Files.createDirectories(out.toAbsolutePath().getParent());
            Files.writeString(out, new GsonBuilder().create().toJson(o));
        } catch (IOException e) {
            throw new IllegalStateException("writing " + out, e);
        }
    }
}
