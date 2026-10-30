package net.ruthenium.gametest;

import net.fabricmc.fabric.api.client.gametest.v1.context.ClientGameTestContext;
import net.fabricmc.fabric.api.client.gametest.v1.context.TestServerContext;
import net.fabricmc.fabric.api.client.gametest.v1.context.TestSingleplayerContext;
import net.minecraft.client.CloudStatus;
import net.minecraft.client.Minecraft;
import net.minecraft.ChatFormatting;
import net.minecraft.core.BlockPos;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.network.chat.Component;
import net.minecraft.world.item.DyeColor;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.entity.SignBlockEntity;
import net.minecraft.world.level.block.entity.SignText;
import net.minecraft.world.level.block.entity.SignTextSlot;
import net.minecraft.world.level.chunk.LevelChunk;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.TreeMap;

/**
 * Block entity benchmark ({@code -Pbench.scene=blockentities}): the block entities that {@code RetainedBlockEntities}
 * keeps besides chests, which the mixed scene ({@link MixedBenchmark}) doesn't have.
 * <p>
 * A fresh flat world (Fabric's consistent test settings) gets a field of ~12.4k block entities in front of a
 * spectator: a row of chests, 6144 shulker boxes (all colours, three facings), 3072 decorated pots (blank and with
 * three sherd sets) and on top of them 3072 skulls (all seven kinds, four rotations, and wall heads). Phases:
 * "static" ({@code bench.staticSeconds}) facing the field and "pan" (a 360 degree pan over {@code bench.panSeconds}).
 * Screenshots: the static view, and with {@code bench.blockEntityEvents=true} a shulker box opened, a pot wobbled and
 * breaking progress on a skull ({@code beevents}), then all of it undone ({@code bereset}). Clouds are off so that
 * screenshots of different runs compare.
 * <p>
 * The events end with glowstone placed above the field and removed ({@code belight}, {@code belightreset}).
 * <p>
 * {@code bench.blockEntitySigns=true} adds a row of signs close to the camera (standing, wall and hanging; plain,
 * coloured, bold and underlined lines, back sides, one glowing), large enough to compare their glyphs; with the
 * events, one sign's text is then changed on the client as the sign edit screen does ({@code besign}).
 * <p>
 * {@code bench.blockEntityKinds=extras} fills the field with other block entities instead (~5.5k): 3072 lecterns with
 * books (four facings), 768 bells on stone (four facings, two layers) and 1536 copper golem statues (four poses, four
 * oxidation levels, four facings); the chest row stays. Its events ring a bell and put breaking progress on a lectern.
 */
final class BlockEntityBenchmark {
    private static final Logger LOGGER = LoggerFactory.getLogger("Ruthenium Block Entity Benchmark");
    private static final int TICKS_PER_SECOND = 20;

    private static final int RENDER_DISTANCE = Integer.parseInt(BenchmarkClientGameTest.prop("blockEntityRenderDistance", "8"));
    private static final float PITCH = 30.0F;
    private static final boolean EVENTS = Boolean.parseBoolean(BenchmarkClientGameTest.prop("blockEntityEvents", "false"));
    private static final boolean SIGNS = Boolean.parseBoolean(BenchmarkClientGameTest.prop("blockEntitySigns", "false"));
    private static final boolean EXTRAS = "extras".equals(BenchmarkClientGameTest.prop("blockEntityKinds", "default"));

    /** The flat world's surface; the field spans x -32..31, z 2..31 */
    private static final int FLOOR = -60;
    private static final String[] SKULLS = {"skeleton_skull", "wither_skeleton_skull", "zombie_head", "creeper_head", "dragon_head",
            "piglin_head", "player_head"};
    private static final String[][] SHERDS = {
            {"angler", "archer", "arms_up", "blade"},
            {"brewer", "burn", "danger", "explorer"},
            {"friend", "heart", "heartbreak", "howl"},
    };

    private static final BlockPos BELL_EVENT = new BlockPos(4, FLOOR + 3, 8);
    private static final BlockPos LECTERN_EVENT = new BlockPos(-6, FLOOR + 3, 8);
    private static final BlockPos SHULKER_EVENT = new BlockPos(-2, FLOOR + 7, 8);
    private static final BlockPos POT_EVENT = new BlockPos(9, FLOOR, 8);
    private static final BlockPos SKULL_EVENT = new BlockPos(2, FLOOR + 7, 8);
    /** The sign row: standing signs on a stone row, wall signs on its north face and hanging signs below it */
    private static final int SIGN_Z = -4;
    private static final int SIGN_ROW_Y = FLOOR + 7;
    private static final BlockPos SIGN_EVENT = new BlockPos(0, SIGN_ROW_Y + 1, SIGN_Z);

    private BlockEntityBenchmark() {
    }

    static void run(ClientGameTestContext context, String label, String runId) {
        context.runOnClient(mc -> {
            mc.options.renderDistance().set(RENDER_DISTANCE);
            mc.options.cloudStatus().set(CloudStatus.OFF);
        });

        List<PhaseResult> results = new ArrayList<>();
        Map<String, String> frameSeries = new HashMap<>();

        try (TestSingleplayerContext world = context.worldBuilder().create()) {
            TestServerContext server = world.getServer();
            server.runCommand("time set 6000");
            server.runCommand("gamemode spectator @a");
            if (EXTRAS) {
                buildExtras(server);
            } else {
                build(server);
            }
            if (SIGNS) {
                buildSigns(server);
            }
            server.runCommand("tp @a 0.5 %d -12.5 0 %s".formatted(FLOOR + 14, PITCH));
            context.waitTicks(TICKS_PER_SECOND);
            TerrainBenchmark.waitForVisibleCompiled(context);
            setView(context, 0.0F);

            Map<String, Integer> counts = context.computeOnClient(BlockEntityBenchmark::countBlockEntities);
            LOGGER.info("Block entities: {}", counts);
            int total = counts.values().stream().mapToInt(Integer::intValue).sum();
            if (total < (EXTRAS ? 5_000 : 12_000)) {
                throw new AssertionError("Only " + total + " block entities were placed: " + counts);
            }

            context.takeScreenshot(label + "-blockentities-static-" + runId);
            if (EVENTS) {
                if (EXTRAS) {
                    extraEvents(context, label, runId);
                } else {
                    events(context, label, runId);
                }
                lightEvents(context, server, label, runId);
            }

            context.runOnClient(_ -> FrameRecorder.startPhase());
            BenchmarkClientGameTest.waitSeconds(context, BenchmarkClientGameTest.STATIC_SECONDS);
            results.add(context.computeOnClient(_ -> FrameRecorder.stopPhase("static", -1)));
            LOGGER.info(results.getLast().summaryLine());

            context.runOnClient(mc -> {
                FrameRecorder.startPhase();
                FrameRecorder.startPan(mc, 0.0F, PITCH, BenchmarkClientGameTest.PAN_SECONDS);
            });
            context.waitFor(_ -> !FrameRecorder.isPanning(), (int) (BenchmarkClientGameTest.PAN_SECONDS * TICKS_PER_SECOND * 10));
            results.add(context.computeOnClient(_ -> FrameRecorder.stopPhase("pan", -1)));
            frameSeries.put("pan", context.computeOnClient(_ -> FrameRecorder.framesCsv()));
            LOGGER.info(results.getLast().summaryLine());

            if (ToggleAb.enabled()) {
                ToggleAb.run(context, results, () -> setView(context, 0.0F), 0.0F, PITCH);
            }

            var environment = context.computeOnClient(BenchmarkClientGameTest::describeEnvironment);
            environment.addProperty("label", label + "-blockentities");
            var blockEntities = new com.google.gson.JsonObject();
            counts.forEach(blockEntities::addProperty);
            environment.add("blockEntities", blockEntities);
            BenchmarkClientGameTest.writeResults(label + "-blockentities", results, environment, frameSeries);
        }
    }

    private static void build(TestServerContext server) {
        int y0 = FLOOR, y1 = FLOOR + 7;
        server.runCommand("fill -32 %d 2 31 %d 3 minecraft:chest[facing=north]".formatted(y0, y0));

        // Shulker boxes: 16 colours x 2 columns x 8 high x 24 deep
        String[] facings = {"up", "north", "east"};
        DyeColor[] colors = DyeColor.values();
        for (int i = 0; i < colors.length; i++) {
            int x = -32 + 2 * i;
            server.runCommand("fill %d %d 8 %d %d 31 minecraft:%s_shulker_box[facing=%s]"
                    .formatted(x, y0, x + 1, y1, colors[i].getName(), facings[i % facings.length]));
        }

        // Decorated pots: 4 groups x 8 wide x 4 high x 24 deep, the first one blank
        String[] potFacings = {"north", "south", "west", "east"};
        for (int g = 0; g < 4; g++) {
            int x = 8 * g;
            String nbt = "";
            if (g > 0) {
                String[] sherds = SHERDS[g - 1];
                nbt = "{sherds:{back:{id:\"minecraft:%s_pottery_sherd\"},left:{id:\"minecraft:%s_pottery_sherd\"},"
                        .formatted(sherds[0], sherds[1])
                        + "right:{id:\"minecraft:%s_pottery_sherd\"},front:{id:\"minecraft:%s_pottery_sherd\"}}}"
                        .formatted(sherds[2], sherds[3]);
            }
            server.runCommand("fill %d %d 8 %d %d 31 minecraft:decorated_pot[facing=%s]%s"
                    .formatted(x, y0, x + 7, y0 + 3, potFacings[g], nbt));
        }

        // Skulls on top of the pots: 7 kinds and zombie wall heads, 4 wide each, one rotation per layer
        for (int k = 0; k <= SKULLS.length; k++) {
            int x = 4 * k;
            for (int layer = 0; layer < 4; layer++) {
                String block = k < SKULLS.length
                        ? "minecraft:%s[rotation=%d]".formatted(SKULLS[k], layer * 4 + k % 4)
                        : "minecraft:zombie_wall_head[facing=%s]".formatted(potFacings[layer]);
                server.runCommand("fill %d %d 8 %d %d 31 %s".formatted(x, y0 + 4 + layer, x + 3, y0 + 4 + layer, block));
            }
        }
    }

    private static void buildExtras(TestServerContext server) {
        int y0 = FLOOR;
        server.runCommand("fill -32 %d 2 31 %d 3 minecraft:chest[facing=north]".formatted(y0, y0));
        String[] facings = {"north", "east", "south", "west"};

        // Lecterns with books: 32 wide x 24 deep x 4 high, one facing per layer
        for (int layer = 0; layer < 4; layer++) {
            server.runCommand("fill -32 %d 8 -1 %d 31 minecraft:lectern[facing=%s,has_book=true]"
                    .formatted(y0 + layer, y0 + layer, facings[layer]));
        }

        // Bells standing on stone: 16 wide x 24 deep, two layers, one facing per 4 columns
        for (int layer = 0; layer < 2; layer++) {
            int y = y0 + 2 * layer;
            server.runCommand("fill 0 %d 8 15 %d 31 minecraft:stone".formatted(y, y));
            for (int f = 0; f < 4; f++) {
                server.runCommand("fill %d %d 8 %d %d 31 minecraft:bell[attachment=floor,facing=%s]"
                        .formatted(4 * f, y + 1, 4 * f + 3, y + 1, facings[f]));
            }
        }

        // Copper golem statues: 16 wide x 24 deep x 4 high, a pose per 4 columns, an oxidation level per layer
        String[] oxidation = {"", "exposed_", "weathered_", "oxidized_"};
        String[] poses = {"standing", "sitting", "running", "star"};
        for (int layer = 0; layer < 4; layer++) {
            for (int p = 0; p < 4; p++) {
                server.runCommand("fill %d %d 8 %d %d 31 minecraft:%scopper_golem_statue[copper_golem_pose=%s,facing=%s]"
                        .formatted(16 + 4 * p, y0 + layer, 16 + 4 * p + 3, y0 + layer, oxidation[layer], poses[p], facings[(p + layer) % 4]));
            }
        }
    }

    /**
     * Glowstone placed above the field (a light change around retained block entities, in sections whose block
     * entities don't change), screenshot ({@code belight}), then removed again ({@code belightreset})
     */
    private static void lightEvents(ClientGameTestContext context, TestServerContext server, String label, String runId) {
        BlockPos pos = EXTRAS ? new BlockPos(-4, FLOOR + 5, 10) : new BlockPos(2, FLOOR + 9, 10);
        server.runCommand("setblock %d %d %d minecraft:glowstone".formatted(pos.getX(), pos.getY(), pos.getZ()));
        context.waitTicks(20);
        context.takeScreenshot(label + "-belight-" + runId);
        server.runCommand("setblock %d %d %d minecraft:air".formatted(pos.getX(), pos.getY(), pos.getZ()));
        context.waitTicks(20);
        context.takeScreenshot(label + "-belightreset-" + runId);
    }

    /** A bell rung and breaking progress on a lectern, client side; screenshots while the bell swings and at rest */
    private static void extraEvents(ClientGameTestContext context, String label, String runId) {
        context.runOnClient(mc -> {
            mc.level.blockEvent(BELL_EVENT, mc.level.getBlockState(BELL_EVENT).getBlock(), 1, net.minecraft.core.Direction.NORTH.get3DDataValue());
            mc.level.destroyBlockProgress(-4244, LECTERN_EVENT, 6);
        });
        context.waitTicks(5);
        context.takeScreenshot(label + "-beevents-" + runId);

        context.runOnClient(mc -> mc.level.destroyBlockProgress(-4244, LECTERN_EVENT, -1));
        // The bell swings for 50 ticks
        context.waitTicks(60);
        context.takeScreenshot(label + "-bereset-" + runId);
    }

    private static Map<String, Integer> countBlockEntities(Minecraft mc) {
        Map<String, Integer> counts = new TreeMap<>();
        BlockPos center = mc.player.blockPosition();
        int chunkX = center.getX() >> 4, chunkZ = center.getZ() >> 4;
        for (int cx = chunkX - 4; cx <= chunkX + 4; cx++) {
            for (int cz = chunkZ - 2; cz <= chunkZ + 6; cz++) {
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

    /** Lid, wobble and breaking progress on block entities in view, client side; screenshots once they are at rest */
    private static void events(ClientGameTestContext context, String label, String runId) {
        context.runOnClient(mc -> {
            mc.level.blockEvent(SHULKER_EVENT, mc.level.getBlockState(SHULKER_EVENT).getBlock(), 1, 1);
            mc.level.blockEvent(POT_EVENT, mc.level.getBlockState(POT_EVENT).getBlock(), 1, 0);
            mc.level.destroyBlockProgress(-4243, SKULL_EVENT, 6);
        });
        // The lid opens in 10 ticks, the wobble lasts 7
        context.waitTicks(30);
        context.takeScreenshot(label + "-beevents-" + runId);

        context.runOnClient(mc -> {
            mc.level.blockEvent(SHULKER_EVENT, mc.level.getBlockState(SHULKER_EVENT).getBlock(), 1, 0);
            mc.level.destroyBlockProgress(-4243, SKULL_EVENT, -1);
        });
        context.waitTicks(30);
        context.takeScreenshot(label + "-bereset-" + runId);

        if (SIGNS) {
            context.runOnClient(mc -> {
                if (mc.level.getBlockEntity(SIGN_EVENT) instanceof SignBlockEntity sign) {
                    SignText text = sign.getText(SignTextSlot.FRONT).asMutable()
                            .setLine(0, Component.literal("Edited"))
                            .setLine(3, Component.literal("on client").withStyle(ChatFormatting.AQUA))
                            .asImmutable();
                    sign.setText(text, SignTextSlot.FRONT);
                } else {
                    throw new AssertionError("No sign at " + SIGN_EVENT);
                }
            });
            context.waitTicks(10);
            context.takeScreenshot(label + "-besign-" + runId);
        }
    }

    private static void buildSigns(TestServerContext server) {
        int y = SIGN_ROW_Y;
        server.runCommand("fill -8 %d %d 8 %d %d minecraft:stone".formatted(y, SIGN_Z, y, SIGN_Z));
        String[] woods = {"oak", "spruce", "birch", "jungle", "acacia", "dark_oak", "mangrove", "cherry", "bamboo", "crimson",
                "warped", "pale_oak"};
        String[] colors = {"black", "red", "white", "blue", "green", "yellow", "purple", "orange", "cyan", "brown", "lime", "gray"};
        for (int x = -8; x <= 8; x++) {
            int i = x + 8;
            String wood = woods[i % woods.length];
            String color = colors[i % colors.length];
            String front = "{messages:[\"Sign %d\",{text:\"Bold %s\",bold:true},{text:\"under\",underlined:true,color:\"gold\"},\"%s\"],color:\"%s\"%s}"
                    .formatted(i, wood, "Wwg@#jy_" + i, color, x == 4 ? ",has_glowing_text:true" : "");
            String back = "{messages:[\"Back %d\",\"\",\"\",\"\"]}".formatted(i);
            // Standing signs facing the camera (north), wall signs on the row's north face, hanging signs below it
            server.runCommand("setblock %d %d %d minecraft:%s_sign[rotation=8]{front_text:%s,back_text:%s}"
                    .formatted(x, y + 1, SIGN_Z, wood, front, back));
            server.runCommand("setblock %d %d %d minecraft:%s_wall_sign[facing=north]{front_text:%s}"
                    .formatted(x, y, SIGN_Z - 1, wood, front));
            if (i % 2 == 0) {
                server.runCommand("setblock %d %d %d minecraft:%s_hanging_sign[rotation=8]{front_text:%s}"
                        .formatted(x, y - 1, SIGN_Z, wood, front));
            }
        }
    }

    private static void setView(ClientGameTestContext context, float yaw) {
        context.runOnClient(mc -> {
            mc.player.setYRot(yaw);
            mc.player.yRotO = yaw;
            mc.player.setXRot(PITCH);
            mc.player.xRotO = PITCH;
        });
        context.waitTicks(10);
    }
}
