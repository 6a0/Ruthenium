package net.vulkanmod.gametest;

import net.fabricmc.fabric.api.client.gametest.v1.context.ClientGameTestContext;
import net.fabricmc.fabric.api.client.gametest.v1.context.TestServerContext;
import net.fabricmc.fabric.api.client.gametest.v1.context.TestSingleplayerContext;
import net.minecraft.client.CloudStatus;
import net.minecraft.client.Minecraft;
import net.minecraft.core.BlockPos;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.world.item.DyeColor;
import net.minecraft.world.level.block.entity.BlockEntity;
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
 * keeps besides chests, which the server scene doesn't have.
 * <p>
 * A fresh flat world (Fabric's consistent test settings) gets a field of ~12.4k block entities in front of a
 * spectator: a row of chests, 6144 shulker boxes (all colours, three facings), 3072 decorated pots (blank and with
 * three sherd sets) and on top of them 3072 skulls (all seven kinds, four rotations, and wall heads). Phases:
 * "static" ({@code bench.staticSeconds}) facing the field and "pan" (a 360 degree pan over {@code bench.panSeconds}).
 * Screenshots: the static view, and with {@code bench.blockEntityEvents=true} a shulker box opened, a pot wobbled and
 * breaking progress on a skull ({@code beevents}), then all of it undone ({@code bereset}). Clouds are off so that
 * screenshots of different runs compare.
 */
final class BlockEntityBenchmark {
    private static final Logger LOGGER = LoggerFactory.getLogger("VulkanMod Block Entity Benchmark");
    private static final int TICKS_PER_SECOND = 20;

    private static final int RENDER_DISTANCE = Integer.parseInt(BenchmarkClientGameTest.prop("blockEntityRenderDistance", "8"));
    private static final float PITCH = 30.0F;
    private static final boolean EVENTS = Boolean.parseBoolean(BenchmarkClientGameTest.prop("blockEntityEvents", "false"));

    /** The flat world's surface; the field spans x -32..31, z 2..31 */
    private static final int FLOOR = -60;
    private static final String[] SKULLS = {"skeleton_skull", "wither_skeleton_skull", "zombie_head", "creeper_head", "dragon_head",
            "piglin_head", "player_head"};
    private static final String[][] SHERDS = {
            {"angler", "archer", "arms_up", "blade"},
            {"brewer", "burn", "danger", "explorer"},
            {"friend", "heart", "heartbreak", "howl"},
    };

    private static final BlockPos SHULKER_EVENT = new BlockPos(-2, FLOOR + 7, 8);
    private static final BlockPos POT_EVENT = new BlockPos(9, FLOOR, 8);
    private static final BlockPos SKULL_EVENT = new BlockPos(2, FLOOR + 7, 8);

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
            build(server);
            server.runCommand("tp @a 0.5 %d -12.5 0 %s".formatted(FLOOR + 14, PITCH));
            context.waitTicks(TICKS_PER_SECOND);
            TerrainBenchmark.waitForVisibleCompiled(context);
            setView(context, 0.0F);

            Map<String, Integer> counts = context.computeOnClient(BlockEntityBenchmark::countBlockEntities);
            LOGGER.info("Block entities: {}", counts);
            int total = counts.values().stream().mapToInt(Integer::intValue).sum();
            if (total < 12_000) {
                throw new AssertionError("Only " + total + " block entities were placed: " + counts);
            }

            context.takeScreenshot(label + "-blockentities-static-" + runId);
            if (EVENTS) {
                events(context, label, runId);
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
