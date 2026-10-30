package net.ruthenium.gametest;

import net.fabricmc.fabric.api.client.gametest.v1.context.ClientGameTestContext;
import net.fabricmc.fabric.api.client.gametest.v1.context.TestServerContext;
import net.fabricmc.fabric.api.client.gametest.v1.context.TestSingleplayerContext;
import net.minecraft.client.Minecraft;
import net.minecraft.core.BlockPos;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.entity.ChestBlockEntity;
import net.minecraft.world.level.chunk.LevelChunk;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.TreeMap;

/**
 * Mixed benchmark ({@code -Pbench.scene=mixed}, the default): a singleplayer stand-in for the multiplayer server
 * scene that the benchmark used until the server went away (2026-10-28). Its numbers don't compare with the old
 * server scene's, only with each other.
 * <p>
 * A fresh flat world (Fabric's consistent test settings) gets the server scene's mix in front of a spectator at
 * render distance 16 ({@code bench.renderDistance}): 296 armor stands ahead (yaw 0), a cube of ~7.2k chests with a
 * layer of ~1k signs inside it to the east (yaw 270), a field of ~5.6k enchanting tables behind the cube (closed
 * books), signs on the cube's face, and slime and honey blocks (translucent terrain). Phases, as on the server:
 * <ol>
 *     <li>"initial": the first {@code bench.initialSeconds} after the teleport, while FPS settles.</li>
 *     <li>"pan": a full 360 degree yaw pan over {@code bench.panSeconds}.</li>
 *     <li>"static": {@code bench.staticSeconds} looking at the particle spot ({@code bench.staticYaw}), as the
 *     reference for the particle phase.</li>
 *     <li>"particles": {@code bench.particleSeconds} after spawning 500k happy_villager particles in view
 *     ({@code bench.particleCommand}).</li>
 * </ol>
 * The initial and pan phases write their per-frame series. Screenshots: yaw 270 (the chest cube;
 * {@code bench.extraShotYaws}) and the static view. {@code bench.zoomShot}
 * adds a 30 degree field of view shot at yaw 270, {@code bench.chestEvents} opens a chest lid and shows breaking
 * progress on another chest before the static phase ({@code chestopen}, {@code chestclosed}).
 * <p>
 * {@code bench.glowingSigns=true} gives the signs glowing text: every other group of the signs inside the cube black
 * (always outlined), the others coloured (outlined only within 16 blocks of the camera entity), the face's signs in turn
 * red, black, lime and light blue (the face's usual dark_red is no dye colour, so it reads as black), and
 * adds yaw 270 screenshots from farther away ({@code glow270x-7}, no face sign in reach) and closer ({@code glow270x4}).
 * It changes the scene, so don't compare its numbers with runs without it.
 * <p>
 * After the phases (once the particles are gone), {@code bench.toggleField=<switch>} runs an in-launch A/B ({@link ToggleAb}), and
 * {@code bench.closeShots=N} takes N frozen close-ups of the armor stands per position ({@code close<k>-<n>}; with
 * {@code bench.closeToggle=<switch>[,<switch>...]} every other shot with the switches flipped, as in the entity scene), clouds off: the grid from the
 * front and from above, and a row of stands summoned for them behind the camera (arms and poses, small, no base plate, invisible,
 * marker) from the front and from above. {@code bench.cloudShots=N} takes N frozen shots of the clouds per view ({@code cloud<k>-<n>},
 * toggled by {@code bench.closeToggle} in the same way): fancy from below, inside and above, fast from below and above, and with improved
 * transparency. Neither changes the measured phases.
 */
final class MixedBenchmark {
    private static final Logger LOGGER = LoggerFactory.getLogger("Ruthenium Mixed Benchmark");
    private static final int TICKS_PER_SECOND = 20;

    private static final double INITIAL_SECONDS = Double.parseDouble(BenchmarkClientGameTest.prop("initialSeconds", "5"));
    private static final double PARTICLE_SECONDS = Double.parseDouble(BenchmarkClientGameTest.prop("particleSeconds", "10"));
    private static final String[] EXTRA_SHOT_YAWS = BenchmarkClientGameTest.prop("extraShotYaws", "270").split(",");
    private static final boolean ZOOM_SHOT = Boolean.parseBoolean(BenchmarkClientGameTest.prop("zoomShot", "false"));
    private static final boolean CHEST_EVENTS = Boolean.parseBoolean(BenchmarkClientGameTest.prop("chestEvents", "false"));
    private static final boolean GLOWING_SIGNS = Boolean.parseBoolean(BenchmarkClientGameTest.prop("glowingSigns", "false"));
    private static final int CLOSE_SHOTS = Integer.parseInt(BenchmarkClientGameTest.prop("closeShots", "0"));
    private static final String CLOSE_TOGGLE = BenchmarkClientGameTest.prop("closeToggle", "");
    private static final int CLOUD_SHOTS = Integer.parseInt(BenchmarkClientGameTest.prop("cloudShots", "0"));
    /** Cloud close-up views: height above the floor, pitch, fancy clouds, improved transparency */
    private static final Object[][] CLOUD_VIEWS = {
            {200.0, -35.0F, true, false},   // below the clouds
            {252.5, 15.0F, true, false},    // inside the cloud layer (y 192.33 to 196.33 in the overworld; interior faces)
            {300.0, 35.0F, true, false},    // above
            {200.0, -35.0F, false, false},  // fast clouds from below
            {300.0, 35.0F, false, false},   // and from above
            {200.0, -35.0F, true, true},    // improved transparency (OIT)
            {252.5, 15.0F, true, true},
            {200.0, -35.0F, false, true},
    };
    /** Close-up camera positions: x, height above the floor, z, yaw, pitch */
    private static final float[][] CLOSE_UPS = {
            {0.5F, 1.7F, 1.5F, 0.0F, 20.0F},     // the grid's first rows: big stands, arms, many yaws
            {0.5F, 4.5F, 3.5F, 0.0F, 55.0F},     // the grid from above, small stands in rows 3 and 7
            {0.0F, 1.5F, -3.5F, 180.0F, 20.0F},  // the summoned variety rows
            {0.0F, 4.0F, -5.0F, 180.0F, 60.0F},  // the same from above
            {0.5F, 2.0F, 0.5F, 0.0F, 0.0F},      // the static view: the chest cube's corner at the right edge
            {0.5F, 2.0F, 0.5F, 270.0F, 0.0F},    // the chest cube
            {6.0F, 3.0F, 20.0F, 240.0F, 10.0F},  // the cube's corner, its sections across the screen edge
    };
    /** Stands summoned for the close-ups behind the camera: x, z, NBT (plates 1.25 blocks apart, so they don't overlap) */
    private static final String[][] VARIETY_STANDS = {
            {"-3.75", "-7.5", "Rotation:[0f,0f]"},
            {"-2.5", "-7.5", "Rotation:[37f,0f],ShowArms:1b"},
            {"-1.25", "-7.5", "Rotation:[90f,0f],ShowArms:1b,Pose:{Head:[15f,30f,0f],Body:[0f,20f,0f],LeftArm:[-90f,0f,0f],"
                    + "RightArm:[-30f,20f,10f],LeftLeg:[-20f,0f,0f],RightLeg:[20f,0f,0f]}"},
            {"0.0", "-7.5", "Rotation:[200f,0f],Small:1b"},
            {"1.25", "-7.5", "Rotation:[315f,0f],Small:1b,ShowArms:1b"},
            {"2.5", "-7.5", "Rotation:[45f,0f],NoBasePlate:1b"},
            {"3.75", "-7.5", "Rotation:[123f,0f],Small:1b,NoBasePlate:1b"},
            {"-3.125", "-10.0", "Rotation:[10f,0f],Invisible:1b"},
            {"-1.875", "-10.0", "Rotation:[270f,0f],Marker:1b"},
            {"-0.625", "-10.0", "Rotation:[160f,0f],Marker:1b,Small:1b,ShowArms:1b"},
            {"0.625", "-10.0", "Rotation:[77f,0f],Small:1b,ShowArms:1b,Pose:{Head:[-20f,0f,10f],RightArm:[-120f,0f,0f],LeftLeg:[10f,0f,5f]}"},
            {"1.875", "-10.0", "Rotation:[11.25f,0f],ShowArms:1b,Pose:{Body:[10f,0f,-5f],LeftArm:[0f,0f,-60f]}"},
            {"3.125", "-10.0", "Rotation:[359f,0f],Invisible:1b,Small:1b"},
    };
    /**
     * Records vanilla's section profiler (the debug profiling report) during the static phase, into
     * {@code debug/profiling/} of the run directory. Times are measured per section, unlike JFR samples, but the
     * profiler slows the frame down, so the static phase's FPS is not comparable with normal runs.
     */
    private static final boolean SECTION_PROFILE = Boolean.parseBoolean(BenchmarkClientGameTest.prop("sectionProfile", "false"));

    /** The flat world's surface */
    private static final int FLOOR = -60;
    /** The spectator's feet, looking along +z at the armor stands with pitch 0 */
    private static final double CAMERA_X = 0.5, CAMERA_Y = FLOOR + 2, CAMERA_Z = 0.5;
    private static final String PARTICLE_COMMAND = BenchmarkClientGameTest.prop("particleCommand",
            "particle minecraft:happy_villager 1.5 %d 10 2 2 2 1 500000".formatted(FLOOR + 1));

    /** The chest cube: x 12..43, z -16..15, FLOOR..FLOOR+7, with the sign layer at SIGN_LAYER */
    private static final int CUBE_MIN_X = 12, CUBE_MAX_X = 43, CUBE_MIN_Z = -16, CUBE_MAX_Z = 15;
    private static final int SIGN_LAYER = FLOOR + 4;
    /** Chests on the cube's west face, in view at yaw 270, for the chest events */
    private static final BlockPos LID_EVENT = new BlockPos(CUBE_MIN_X, FLOOR + 2, -1);
    private static final BlockPos BREAK_EVENT = new BlockPos(CUBE_MIN_X, FLOOR + 2, 2);
    /** The enchanting table field behind the cube: 75 x 75 */
    private static final int TABLES_MIN_X = 48, TABLES_MIN_Z = -37, TABLES_SIZE = 75;

    private static final String[] WOODS = {"oak", "spruce", "birch", "jungle", "acacia", "dark_oak", "mangrove", "cherry"};
    /** Text colours of the glowing sign groups inside the cube (with glowingSigns) */
    private static final String[] GLOW_COLORS = {"black", "light_blue", "black", "yellow", "black", "lime", "black", "red"};
    private static final String[] FACE_GLOW_COLORS = {"red", "black", "lime", "light_blue"};

    private MixedBenchmark() {
    }

    static void run(ClientGameTestContext context, String label, String runId) {
        String sceneLabel = label + "-mixed";
        List<PhaseResult> results = new ArrayList<>();
        Map<String, String> frameSeries = new HashMap<>();

        try (TestSingleplayerContext world = context.worldBuilder().create()) {
            TestServerContext server = world.getServer();
            server.runCommand("time set 6000");
            server.runCommand("weather clear");
            server.runCommand("gamemode spectator @a");
            build(server);

            server.runCommand("tp @a %s %s %s 0 0".formatted(CAMERA_X, CAMERA_Y, CAMERA_Z));
            context.waitFor(mc -> mc.player != null && mc.player.distanceToSqr(CAMERA_X, CAMERA_Y, CAMERA_Z) < 0.01,
                    10 * TICKS_PER_SECOND);

            // Initial phase: measure while FPS settles after the teleport (chunks and block entities still arriving)
            context.runOnClient(_ -> FrameRecorder.startPhase());
            BenchmarkClientGameTest.waitSeconds(context, INITIAL_SECONDS);
            results.add(context.computeOnClient(_ -> FrameRecorder.stopPhase("initial", -1)));
            frameSeries.put("initial", context.computeOnClient(_ -> FrameRecorder.framesCsv()));
            LOGGER.info(results.getLast().summaryLine());

            TerrainBenchmark.waitForVisibleCompiled(context);
            Map<String, Integer> blockEntities = context.computeOnClient(MixedBenchmark::countBlockEntities);
            Map<String, Integer> entities = context.computeOnClient(MixedBenchmark::countEntities);
            LOGGER.info("Block entities: {}, entities: {}", blockEntities, entities);
            int totalBlockEntities = blockEntities.values().stream().mapToInt(Integer::intValue).sum();
            if (totalBlockEntities < 13_000 || entities.getOrDefault("armor_stand", 0) < 296) {
                throw new AssertionError("The scene is incomplete: " + blockEntities + ", " + entities);
            }

            // 360 pan
            context.runOnClient(mc -> {
                FrameRecorder.startPhase();
                FrameRecorder.startPan(mc, 0.0F, BenchmarkClientGameTest.PAN_SECONDS);
            });
            context.waitFor(_ -> !FrameRecorder.isPanning(), (int) (BenchmarkClientGameTest.PAN_SECONDS * TICKS_PER_SECOND * 10));
            results.add(context.computeOnClient(_ -> FrameRecorder.stopPhase("pan", -1)));
            frameSeries.put("pan", context.computeOnClient(_ -> FrameRecorder.framesCsv()));
            LOGGER.info(results.getLast().summaryLine());

            // Static view at the particle spot, as the reference for the particle phase.
            // The screenshots (taken before measuring) are for comparing rendering output between configurations.
            for (String yaw : EXTRA_SHOT_YAWS) {
                if (!yaw.isBlank()) {
                    BenchmarkClientGameTest.setYaw(context, Float.parseFloat(yaw.trim()));
                    context.takeScreenshot(label + "-yaw" + (int) Float.parseFloat(yaw.trim()) + "-" + runId);
                }
            }
            if (ZOOM_SHOT) {
                int fov = context.computeOnClient(mc -> mc.options.fov().get());
                context.runOnClient(mc -> mc.options.fov().set(30));
                BenchmarkClientGameTest.setYaw(context, 270.0F);
                context.takeScreenshot(label + "-zoom270-" + runId);
                context.runOnClient(mc -> mc.options.fov().set(fov));
            }
            if (GLOWING_SIGNS) {
                glowShots(context, server, label, runId);
            }
            BenchmarkClientGameTest.setYaw(context, BenchmarkClientGameTest.STATIC_YAW);

            context.takeScreenshot(label + "-static-" + runId);
            if (CHEST_EVENTS) {
                chestEvents(context, label, runId);
            }
            context.runOnClient(_ -> FrameRecorder.startPhase());
            if (SECTION_PROFILE) {
                context.runOnClient(mc -> mc.debugClientMetricsStart(message -> LOGGER.info(message.getString())));
            }
            BenchmarkClientGameTest.waitSeconds(context, BenchmarkClientGameTest.STATIC_SECONDS);
            if (SECTION_PROFILE) {
                // Toggles the recording off; the report is written asynchronously during the particle phase
                context.runOnClient(mc -> mc.debugClientMetricsStart(message -> LOGGER.info(message.getString())));
            }
            results.add(context.computeOnClient(_ -> FrameRecorder.stopPhase("static", -1)));
            LOGGER.info(results.getLast().summaryLine());

            // Particle stress test: the integrated server sends the particles in packets, as a dedicated server would
            context.runOnClient(_ -> FrameRecorder.startPhase());
            server.runCommand(PARTICLE_COMMAND);
            int peakParticles = 0;
            long end = System.nanoTime() + (long) (PARTICLE_SECONDS * 1.0e9);
            while (System.nanoTime() < end) {
                context.waitTicks(5);
                peakParticles = Math.max(peakParticles, context.computeOnClient(BenchmarkClientGameTest::countParticles));
            }
            // The group fills to ~16.2k; one run of the first build only reached 1625 (not reproduced), which would
            // have skewed the phase
            if (peakParticles < 12_000) {
                throw new AssertionError("The particle command didn't fill the particle group (peak " + peakParticles + " particles)");
            }
            int finalPeak = peakParticles;
            results.add(context.computeOnClient(_ -> FrameRecorder.stopPhase("particles", finalPeak)));
            LOGGER.info(results.getLast().summaryLine());

            if (ToggleAb.enabled() || CLOSE_SHOTS > 0 || CLOUD_SHOTS > 0) {
                // The particles are around the armor stands
                context.waitFor(mc -> BenchmarkClientGameTest.countParticles(mc) == 0, 60 * TICKS_PER_SECOND);
            }
            if (ToggleAb.enabled()) {
                ToggleAb.run(context, results, () -> BenchmarkClientGameTest.setYaw(context, BenchmarkClientGameTest.STATIC_YAW), 0.0F, 0.0F);
            }
            if (CLOSE_SHOTS > 0) {
                closeUps(context, server, label, runId);
            }
            if (CLOUD_SHOTS > 0) {
                cloudShots(context, server, label, runId);
            }

            var environment = context.computeOnClient(BenchmarkClientGameTest::describeEnvironment);
            environment.addProperty("label", sceneLabel);
            var blockEntityCounts = new com.google.gson.JsonObject();
            blockEntities.forEach(blockEntityCounts::addProperty);
            environment.add("blockEntities", blockEntityCounts);
            BenchmarkClientGameTest.writeResults(sceneLabel, results, environment, frameSeries);
        }
    }

    private static void build(TestServerContext server) {
        // Commands only reach loaded chunks; the field reaches beyond the spawn chunks
        server.runCommand("forceload add -24 %d %d %d".formatted(CUBE_MIN_Z - 24, TABLES_MIN_X + TABLES_SIZE, TABLES_MIN_Z + TABLES_SIZE));

        // Chest cube, with a layer of standing signs inside (their text drawn through the gaps between chests)
        for (int y = FLOOR; y <= FLOOR + 7; y++) {
            if (y != SIGN_LAYER) {
                server.runCommand("fill %d %d %d %d %d %d minecraft:chest[facing=west]"
                        .formatted(CUBE_MIN_X, y, CUBE_MIN_Z, CUBE_MAX_X, y, CUBE_MAX_Z));
            }
        }
        for (int i = 0; i < WOODS.length; i++) {
            int x0 = CUBE_MIN_X + 4 * i;
            server.runCommand("fill %d %d %d %d %d %d minecraft:%s_sign[rotation=4]{front_text:%s,back_text:%s}"
                    .formatted(x0, SIGN_LAYER, CUBE_MIN_Z, x0 + 3, SIGN_LAYER, CUBE_MAX_Z, WOODS[i],
                            signText("Inside " + i, GLOWING_SIGNS ? GLOW_COLORS[i] : "black", GLOWING_SIGNS),
                            signText("Back " + i, "dark_blue", false)));
        }
        // Standing signs on the ground along the cube's west face, facing the camera at yaw 270
        for (int z = CUBE_MIN_Z; z <= CUBE_MAX_Z; z += 2) {
            server.runCommand("setblock %d %d %d minecraft:%s_sign[rotation=4]{front_text:%s}"
                    .formatted(CUBE_MIN_X - 1, FLOOR, z, WOODS[Math.floorMod(z, WOODS.length)], signText("Face " + z,
                            GLOWING_SIGNS ? FACE_GLOW_COLORS[Math.floorMod(z / 2, FACE_GLOW_COLORS.length)] : "dark_red", GLOWING_SIGNS)));
        }

        // Enchanting tables: far enough that their books stay closed
        server.runCommand("fill %d %d %d %d %d %d minecraft:enchanting_table".formatted(TABLES_MIN_X, FLOOR, TABLES_MIN_Z,
                TABLES_MIN_X + TABLES_SIZE - 1, FLOOR, TABLES_MIN_Z + TABLES_SIZE - 1));

        // Slime and honey blocks to the west (translucent terrain)
        server.runCommand("fill -20 %d -8 -12 %d 8 minecraft:slime_block".formatted(FLOOR, FLOOR + 3));
        server.runCommand("fill -30 %d -8 -22 %d 8 minecraft:honey_block".formatted(FLOOR, FLOOR + 3));

        // 296 armor stands ahead: 37 x 8, some with arms, some small
        for (int row = 0; row < 8; row++) {
            for (int col = 0; col < 37; col++) {
                double x = -17.5 + col, z = 4.5 + 2 * row;
                String nbt = "{NoGravity:1b,Invulnerable:1b,Rotation:[%df,0f]%s%s}".formatted(
                        (col * 23) % 360, col % 3 == 0 ? ",ShowArms:1b" : "", row % 4 == 3 ? ",Small:1b" : "");
                server.runCommand("summon minecraft:armor_stand %s %d %s %s".formatted(x, FLOOR, z, nbt));
            }
        }

        server.runCommand("forceload remove all");
    }

    /** Frozen close-ups of the armor stands, clouds off (after the measured phases) */
    private static void closeUps(ClientGameTestContext context, TestServerContext server, String label, String runId) {
        for (String[] stand : VARIETY_STANDS) {
            server.runCommand("summon minecraft:armor_stand %s %d %s {NoGravity:1b,Invulnerable:1b,%s}".formatted(stand[0], FLOOR, stand[1], stand[2]));
        }
        context.runOnClient(mc -> mc.options.cloudStatus().set(net.minecraft.client.CloudStatus.OFF));
        context.waitTicks(10);
        server.runCommand("tick freeze");
        context.waitTicks(5);
        for (int k = 0; k < CLOSE_UPS.length; k++) {
            float[] view = CLOSE_UPS[k];
            server.runCommand("tp @a %s %s %s %s %s".formatted(view[0], FLOOR + view[1], view[2], view[3], view[4]));
            context.waitTicks(10);
            context.runOnClient(mc -> {
                mc.player.setYRot(view[3]);
                mc.player.yRotO = view[3];
                mc.player.setXRot(view[4]);
                mc.player.xRotO = view[4];
            });
            context.waitTicks(5);
            for (int n = 0; n < CLOSE_SHOTS; n++) {
                boolean toggled = !CLOSE_TOGGLE.isEmpty() && n % 2 == 1;
                if (toggled) {
                    toggleConfig(context);
                }
                context.takeScreenshot("%s-mixed-close%d-%d-%s".formatted(label, k, n, runId));
                if (toggled) {
                    toggleConfig(context);
                }
            }
        }
        server.runCommand("tick unfreeze");
    }

    /** Frozen shots of the clouds (after the measured phases), each view with its cloud and transparency settings */
    private static void cloudShots(ClientGameTestContext context, TestServerContext server, String label, String runId) {
        server.runCommand("tick freeze");
        context.waitTicks(5);
        for (int k = 0; k < CLOUD_VIEWS.length; k++) {
            Object[] view = CLOUD_VIEWS[k];
            double height = (Double) view[0];
            float pitch = (Float) view[1];
            boolean fancy = (Boolean) view[2];
            boolean oit = (Boolean) view[3];
            context.runOnClient(mc -> {
                mc.options.cloudStatus().set(fancy ? net.minecraft.client.CloudStatus.FANCY : net.minecraft.client.CloudStatus.FAST);
                if (mc.options.improvedTransparency().get() != oit) {
                    mc.options.improvedTransparency().set(oit);
                }
            });
            server.runCommand("tp @a %s %s %s 30 %s".formatted(CAMERA_X, FLOOR + height, CAMERA_Z, pitch));
            context.waitTicks(20);
            context.runOnClient(mc -> {
                mc.player.setYRot(30.0F);
                mc.player.yRotO = 30.0F;
                mc.player.setXRot(pitch);
                mc.player.xRotO = pitch;
            });
            context.waitTicks(5);
            LOGGER.info("Cloud view {}: y {}, pitch {}, {}{}", k, FLOOR + height, pitch, fancy ? "fancy" : "fast", oit ? ", improved transparency" : "");
            for (int n = 0; n < CLOUD_SHOTS; n++) {
                boolean toggled = !CLOSE_TOGGLE.isEmpty() && n % 2 == 1;
                if (toggled) {
                    toggleConfig(context);
                }
                context.takeScreenshot("%s-mixed-cloud%d-%d-%s".formatted(label, k, n, runId));
                if (toggled) {
                    toggleConfig(context);
                }
            }
        }
        context.runOnClient(mc -> {
            mc.options.cloudStatus().set(net.minecraft.client.CloudStatus.FANCY);
            if (mc.options.improvedTransparency().get()) {
                mc.options.improvedTransparency().set(false);
            }
        });
        server.runCommand("tp @a %s %s %s 0 0".formatted(CAMERA_X, CAMERA_Y, CAMERA_Z));
        server.runCommand("tick unfreeze");
    }

    /**
     * Flips the boolean {@code Config} fields named by {@code bench.closeToggle} (comma separated), by reflection (vanilla runs
     * have no mod)
     */
    private static void toggleConfig(ClientGameTestContext context) {
        context.runOnClient(_ -> {
            try {
                Object config = Class.forName("net.ruthenium.Initializer").getField("CONFIG").get(null);
                for (String name : CLOSE_TOGGLE.split(",")) {
                    var field = config.getClass().getField(name.trim());
                    field.setBoolean(config, !field.getBoolean(config));
                }
            } catch (ReflectiveOperationException e) {
                throw new AssertionError("Can't toggle " + CLOSE_TOGGLE, e);
            }
        });
        // Switches that drop kept geometry (retainedCellCulling) rebuild it over a few frames
        context.waitTicks(10);
    }

    private static String signText(String first, String color, boolean glowing) {
        return "{messages:[\"%s\",\"Ruthenium\",\"mixed scene\",\"0123456789\"],color:\"%s\"%s}".formatted(first, color,
                glowing ? ",has_glowing_text:true" : "");
    }

    /**
     * Yaw 270 screenshots from 7.5 blocks farther from the cube (no face sign within 16 blocks of the camera entity) and
     * 4 blocks closer (more of them), then back at the start
     */
    private static void glowShots(ClientGameTestContext context, TestServerContext server, String label, String runId) {
        for (double x : new double[]{CAMERA_X - 7.0, CAMERA_X + 4.0, CAMERA_X}) {
            server.runCommand("tp @a %s %s %s 270 0".formatted(x, CAMERA_Y, CAMERA_Z));
            context.waitFor(mc -> mc.player != null && mc.player.distanceToSqr(x, CAMERA_Y, CAMERA_Z) < 0.01, 10 * TICKS_PER_SECOND);
            BenchmarkClientGameTest.setYaw(context, 270.0F);
            if (x != CAMERA_X) {
                context.takeScreenshot(label + "-glow270x" + (int) Math.floor(x) + "-" + runId);
            }
        }
    }

    /** Opens a chest lid and shows breaking progress on another chest (client side), screenshots open and closed */
    private static void chestEvents(ClientGameTestContext context, String label, String runId) {
        BenchmarkClientGameTest.setYaw(context, 270.0F);
        LOGGER.info("Chest events: lid at {}, breaking progress at {}", LID_EVENT, BREAK_EVENT);
        context.runOnClient(mc -> {
            mc.level.blockEvent(LID_EVENT, mc.level.getBlockState(LID_EVENT).getBlock(), 1, 1);
            mc.level.destroyBlockProgress(-4242, BREAK_EVENT, 6);
        });
        context.waitTicks(30);
        context.takeScreenshot(label + "-chestopen-" + runId);
        // The lid only moves while its ticker runs, so this also checks that a parked ticker was woken
        assertLid(context, LID_EVENT, 1.0F);

        context.runOnClient(mc -> {
            mc.level.blockEvent(LID_EVENT, mc.level.getBlockState(LID_EVENT).getBlock(), 1, 0);
            mc.level.destroyBlockProgress(-4242, BREAK_EVENT, -1);
        });
        context.waitTicks(40);
        context.takeScreenshot(label + "-chestclosed-" + runId);
        assertLid(context, LID_EVENT, 0.0F);
        BenchmarkClientGameTest.setYaw(context, BenchmarkClientGameTest.STATIC_YAW);
    }

    private static void assertLid(ClientGameTestContext context, BlockPos pos, float expected) {
        float openness = context.computeOnClient(mc -> mc.level.getBlockEntity(pos) instanceof ChestBlockEntity chest
                ? chest.getOpenNess(1.0F) : Float.NaN);
        LOGGER.info("Chest lid at {}: openness {}", pos, openness);
        if (openness != expected) {
            throw new AssertionError("Chest lid at " + pos + " has openness " + openness + ", expected " + expected);
        }
    }

    private static Map<String, Integer> countBlockEntities(Minecraft mc) {
        Map<String, Integer> counts = new TreeMap<>();
        for (int cx = -2; cx <= (TABLES_MIN_X + TABLES_SIZE) >> 4; cx++) {
            for (int cz = (TABLES_MIN_Z >> 4) - 1; cz <= (TABLES_MIN_Z + TABLES_SIZE) >> 4; cz++) {
                LevelChunk chunk = mc.level.getChunkSource().getChunk(cx, cz, false);
                if (chunk == null) {
                    continue;
                }
                for (BlockEntity blockEntity : chunk.getBlockEntities().values()) {
                    counts.merge(BuiltInRegistries.BLOCK_ENTITY_TYPE.getKey(blockEntity.getType()).getPath(), 1, Integer::sum);
                }
            }
        }
        return counts;
    }

    private static Map<String, Integer> countEntities(Minecraft mc) {
        Map<String, Integer> counts = new TreeMap<>();
        for (Entity entity : mc.level.entitiesForRendering()) {
            counts.merge(BuiltInRegistries.ENTITY_TYPE.getKey(entity.getType()).getPath(), 1, Integer::sum);
        }
        return counts;
    }
}
