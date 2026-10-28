package net.thallium.gametest;

import net.fabricmc.fabric.api.client.gametest.v1.context.ClientGameTestContext;
import net.fabricmc.fabric.api.client.gametest.v1.context.TestServerContext;
import net.fabricmc.fabric.api.client.gametest.v1.context.TestSingleplayerContext;
import net.minecraft.client.CloudStatus;
import net.minecraft.client.Minecraft;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.world.entity.Entity;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.TreeMap;

/**
 * Entity benchmark ({@code -Pbench.scene=entities}): the entity types that the server scene (armor stands only)
 * doesn't have.
 * <p>
 * A fresh flat world (Fabric's consistent test settings) gets ~1.3k entities in rows in front of a spectator, near to
 * far: dropped items, experience orbs, farm animals (some babies), villagers and zombie villagers, zombies and
 * skeletons (with unbreakable helmets, so they don't burn), creepers, spiders, endermen, illagers and witches, iron
 * golems, horses, wolves and cats, minecarts and boats, text/item/block displays, and a wall with item frames and
 * paintings (armor stands are in the server scene). Some mobs have visible name tags. Mobs have no AI unless {@code bench.entityAi=true},
 * so that runs compare. Phases: "static" ({@code bench.staticSeconds}) facing the field and "pan" (a 360 degree pan
 * over {@code bench.panSeconds}). Screenshot: the static view. Clouds are off so that screenshots of different runs
 * compare.
 * <p>
 * {@code bench.closeShots=N} adds close-ups of the mob rows after the benchmark, with ticks frozen so that frames of
 * one run differ only where rendering isn't deterministic: N screenshots per position ({@code close<k>-<n>}). With
 * {@code bench.closeToggle=<Config field>} the boolean switch is flipped before every other screenshot (and restored
 * after), so that shots 0, 2, ... and 1, 3, ... compare the two settings on the same frozen frame.
 */
final class EntityBenchmark {
    private static final Logger LOGGER = LoggerFactory.getLogger("Thallium Entity Benchmark");
    private static final int TICKS_PER_SECOND = 20;

    private static final int RENDER_DISTANCE = Integer.parseInt(BenchmarkClientGameTest.prop("entityRenderDistance", "12"));
    private static final float PITCH = 30.0F;
    private static final boolean AI = Boolean.parseBoolean(BenchmarkClientGameTest.prop("entityAi", "false"));
    /** {@code bench.entityGravity=true}: dropped items and experience orbs fall and rest on the floor (as in real play) */
    private static final boolean GRAVITY = Boolean.parseBoolean(BenchmarkClientGameTest.prop("entityGravity", "false"));
    private static final int CLOSE_SHOTS = Integer.parseInt(BenchmarkClientGameTest.prop("closeShots", "0"));
    private static final String CLOSE_TOGGLE = BenchmarkClientGameTest.prop("closeToggle", "");
    /** {@code bench.staticView=k}: the static phase runs at close-up position k instead of the overview (pan unchanged) */
    private static final int STATIC_VIEW = Integer.parseInt(BenchmarkClientGameTest.prop("staticView", "-1"));
    private static final boolean LIGHT_EVENTS =Boolean.parseBoolean(BenchmarkClientGameTest.prop("lightEvents", "false"));

    /** Close-up camera positions (x, height above the floor, z, pitch), each looking along +z at a few mob rows */
    private static final float[][] CLOSE_UPS = {
            {0.5F, 1.8F, 4.5F, 15.0F},   // farm animals
            {0.5F, 2.2F, 11.0F, 20.0F},  // villagers, zombie villagers
            {0.5F, 2.2F, 15.5F, 20.0F},  // zombies, skeletons, creepers
            {0.5F, 2.6F, 20.5F, 20.0F},  // spiders, endermen
            {0.5F, 2.6F, 24.0F, 20.0F},  // illagers, iron golems
            {0.5F, 2.6F, 28.0F, 20.0F},  // horses, wolves and cats
            {0.5F, 2.0F, 32.0F, 15.0F},  // vehicles, displays, item frames
            {0.5F, 1.6F, -2.0F, 25.0F},  // dropped items, experience orbs
    };

    /** The flat world's surface; the field spans x -32..31, z 0..37 */
    private static final int FLOOR = -60;
    private static final int MIN_X = -32, MAX_X = 32;
    private static final int WALL_Z = 37;

    private static final String[] ITEMS = {"diamond_sword", "stone", "oak_log", "apple", "bow", "iron_ingot", "torch", "glass",
            "redstone", "egg", "cobblestone", "golden_carrot", "shield", "bone", "oak_sapling", "rotten_flesh"};
    private static final String[] PROFESSIONS = {"farmer", "librarian", "armorer", "butcher", "cartographer", "cleric", "fisherman",
            "fletcher", "leatherworker", "mason", "shepherd", "toolsmith", "weaponsmith", "nitwit", "none"};
    private static final String[] VILLAGER_TYPES = {"plains", "desert", "jungle", "savanna", "snow", "swamp", "taiga"};
    private static final String[] HELMETS = {"leather_helmet", "iron_helmet", "golden_helmet", "diamond_helmet", "chainmail_helmet"};
    private static final String[] DISPLAY_BLOCKS = {"oak_stairs", "lantern", "glass", "chest", "stone_bricks", "campfire"};

    private EntityBenchmark() {
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
            server.runCommand("difficulty normal");
            server.runCommand("gamemode spectator @a");
            build(server);
            server.runCommand("tp @a 0.5 %d -8.5 0 %s".formatted(FLOOR + 14, PITCH));
            context.waitTicks(TICKS_PER_SECOND);
            TerrainBenchmark.waitForVisibleCompiled(context);
            setView(context, 0.0F);
            // Entities arrive with their chunks; give the last ones time to come in
            context.waitTicks(2 * TICKS_PER_SECOND);

            Map<String, Integer> counts = context.computeOnClient(EntityBenchmark::countEntities);
            LOGGER.info("Entities: {}", counts);
            int total = counts.values().stream().mapToInt(Integer::intValue).sum();
            // With AI, mobs fight while the chunks arrive (zombies and villagers, golems, wolves and skeletons)
            if (total < (AI ? 1_200 : 1_300)) {
                throw new AssertionError("Only " + total + " entities were summoned: " + counts);
            }

            context.takeScreenshot(label + "-entities-static-" + runId);

            if (STATIC_VIEW >= 0) {
                moveTo(context, server, CLOSE_UPS[STATIC_VIEW]);
            }
            context.runOnClient(_ -> FrameRecorder.startPhase());
            BenchmarkClientGameTest.waitSeconds(context, BenchmarkClientGameTest.STATIC_SECONDS);
            results.add(context.computeOnClient(_ -> FrameRecorder.stopPhase("static", -1)));
            LOGGER.info(results.getLast().summaryLine());
            if (STATIC_VIEW >= 0) {
                server.runCommand("tp @a 0.5 %d -8.5 0 %s".formatted(FLOOR + 14, PITCH));
                context.waitTicks(10);
                setView(context, 0.0F);
            }

            context.runOnClient(mc -> {
                FrameRecorder.startPhase();
                FrameRecorder.startPan(mc, 0.0F, PITCH, BenchmarkClientGameTest.PAN_SECONDS);
            });
            context.waitFor(_ -> !FrameRecorder.isPanning(), (int) (BenchmarkClientGameTest.PAN_SECONDS * TICKS_PER_SECOND * 10));
            results.add(context.computeOnClient(_ -> FrameRecorder.stopPhase("pan", -1)));
            frameSeries.put("pan", context.computeOnClient(_ -> FrameRecorder.framesCsv()));
            LOGGER.info(results.getLast().summaryLine());

            if (LIGHT_EVENTS) {
                lightEvents(context, server);
            }
            if (CLOSE_SHOTS > 0) {
                closeUps(context, server, label, runId);
            }

            var environment = context.computeOnClient(BenchmarkClientGameTest::describeEnvironment);
            environment.addProperty("label", label + "-entities");
            environment.addProperty("entityAi", AI);
            var entities = new com.google.gson.JsonObject();
            counts.forEach(entities::addProperty);
            environment.add("entities", entities);
            BenchmarkClientGameTest.writeResults(label + "-entities", results, environment, frameSeries);
        }
    }

    /**
     * {@code bench.lightEvents=true}: after the pan, glowstone blocks are placed above the mob rows and removed again,
     * in turn, so that the block and sky light at the mobs changes while they are drawn (for checking light kept across
     * frames). Not measured.
     */
    private static void lightEvents(ClientGameTestContext context, TestServerContext server) {
        for (int round = 0; round < 8; round++) {
            String block = round % 2 == 0 ? "glowstone" : "air";
            for (int x = MIN_X + 2; x < MAX_X; x += 5) {
                for (int z = 8; z <= 32; z += 3) {
                    server.runCommand("setblock %d %d %d minecraft:%s".formatted(x, FLOOR + 3, z, block));
                }
            }
            context.waitTicks(30);
        }
        LOGGER.info("Light events done");
    }

    private static void closeUps(ClientGameTestContext context, TestServerContext server, String label, String runId) {
        server.runCommand("tick freeze");
        context.waitTicks(5);
        for (int k = 0; k < CLOSE_UPS.length; k++) {
            moveTo(context, server, CLOSE_UPS[k]);
            for (int n = 0; n < CLOSE_SHOTS; n++) {
                boolean toggled = !CLOSE_TOGGLE.isEmpty() && n % 2 == 1;
                if (toggled) {
                    toggleConfig(context);
                }
                context.takeScreenshot("%s-entities-close%d-%d-%s".formatted(label, k, n, runId));
                if (toggled) {
                    toggleConfig(context);
                }
            }
        }
        server.runCommand("tick unfreeze");
    }

    /** Moves the spectator to a close-up position (x, height above the floor, z, pitch), looking along +z */
    private static void moveTo(ClientGameTestContext context, TestServerContext server, float[] view) {
        server.runCommand("tp @a %s %s %s 0 %s".formatted(view[0], FLOOR + view[1], view[2], view[3]));
        context.waitTicks(10);
        context.runOnClient(mc -> {
            mc.player.setYRot(0.0F);
            mc.player.yRotO = 0.0F;
            mc.player.setXRot(view[3]);
            mc.player.xRotO = view[3];
        });
        context.waitTicks(5);
    }

    /** Flips the boolean {@code Config} field named by {@code bench.closeToggle}, by reflection (vanilla runs have no mod) */
    private static void toggleConfig(ClientGameTestContext context) {
        context.runOnClient(_ -> {
            try {
                Object config = Class.forName("net.thallium.Initializer").getField("CONFIG").get(null);
                var field = config.getClass().getField(CLOSE_TOGGLE);
                field.setBoolean(config, !field.getBoolean(config));
            } catch (ReflectiveOperationException e) {
                throw new AssertionError("Can't toggle " + CLOSE_TOGGLE, e);
            }
        });
    }

    private static void build(TestServerContext server) {
        String mob = AI ? "PersistenceRequired:1b" : "NoAI:1b,PersistenceRequired:1b";
        String facing = "Rotation:[180f,0f]";

        // Dropped items: 4 rows, 1 block apart (items closer than that merge)
        for (int row = 0; row < 4; row++) {
            for (int x = MIN_X; x < MAX_X; x++) {
                String item = ITEMS[Math.floorMod(x + row * 5, ITEMS.length)];
                summon(server, "item", x + 0.5, FLOOR + 0.5, row + 0.5,
                        "Item:{id:\"minecraft:%s\",count:1}%s,PickupDelay:32767,Age:-32768".formatted(item, GRAVITY ? "" : ",NoGravity:1b"));
            }
        }

        // Experience orbs with different values (orbs of the same value merge)
        for (int x = MIN_X; x < MAX_X; x++) {
            summon(server, "experience_orb", x + 0.5, FLOOR + 0.5, 5.5, "Value:%d%s,Age:-32768".formatted(x - MIN_X + 1, GRAVITY ? "" : ",NoGravity:1b"));
        }

        // Farm animals, every fourth a baby
        String[] animals = {"cow", "pig", "sheep", "chicken"};
        for (int a = 0; a < animals.length; a++) {
            double z = 7.5 + 1.5 * a;
            int i = 0;
            for (double x = MIN_X + 0.5; x < MAX_X; x += 1.5, i++) {
                StringBuilder nbt = new StringBuilder(mob).append(',').append(facing);
                if (i % 4 == 3) {
                    nbt.append(",Age:-2000000000");
                }
                if (animals[a].equals("sheep")) {
                    nbt.append(",Color:%db".formatted(i % 16));
                    if (i % 5 == 4) {
                        nbt.append(",Sheared:1b");
                    }
                }
                if (i % 9 == 0) {
                    nbt.append(",CustomName:\"%s %d\",CustomNameVisible:1b".formatted(animals[a], i));
                }
                summon(server, animals[a], x, FLOOR, z, nbt.toString());
            }
        }

        // Villagers and zombie villagers with all professions and biome types
        for (int v = 0; v < 2; v++) {
            int i = 0;
            for (double x = MIN_X + 0.5; x < MAX_X; x += 1.5, i++) {
                String data = "VillagerData:{profession:\"minecraft:%s\",level:%d,type:\"minecraft:%s\"}"
                        .formatted(PROFESSIONS[i % PROFESSIONS.length], 1 + i % 5, VILLAGER_TYPES[i % VILLAGER_TYPES.length]);
                String nbt = mob + "," + facing + "," + data;
                if (v == 1) {
                    nbt += "," + equipment(helmet(i), null);
                }
                summon(server, v == 0 ? "villager" : "zombie_villager", x, FLOOR, 13.5 + 1.5 * v, nbt);
            }
        }

        // Zombies (every third in full armor with a sword: trimmed chestplates, two patterns and materials, every other
        // armored one with enchanted leggings) and skeletons with bows
        int i = 0;
        for (double x = MIN_X + 0.5; x < MAX_X; x += 1.5, i++) {
            String trim = i % 6 == 0 ? "{material:\"minecraft:gold\",pattern:\"minecraft:bolt\"}"
                    : "{material:\"minecraft:diamond\",pattern:\"minecraft:sentry\"}";
            String legs = i % 6 == 0 ? "{id:\"minecraft:iron_leggings\",count:1,components:{\"minecraft:enchantments\":{\"minecraft:protection\":1}}}"
                    : "{id:\"minecraft:iron_leggings\",count:1}";
            String zombieEquipment = i % 3 == 0
                    ? "equipment:{head:%s,chest:{id:\"minecraft:iron_chestplate\",count:1,components:{\"minecraft:trim\":%s}},legs:%s,feet:{id:\"minecraft:iron_boots\",count:1},mainhand:{id:\"minecraft:iron_sword\",count:1}}"
                            .formatted(unbreakable("iron_helmet"), trim, legs)
                    : equipment(helmet(i), null);
            summon(server, "zombie", x, FLOOR, 17.5, mob + "," + facing + "," + zombieEquipment);
            summon(server, "skeleton", x, FLOOR, 19.0, mob + "," + facing + "," + equipment(helmet(i + 2), "bow"));
        }

        // Creepers (every eighth charged), spiders and endermen holding blocks
        i = 0;
        for (double x = MIN_X + 0.5; x < MAX_X; x += 1.5, i++) {
            summon(server, "creeper", x, FLOOR, 20.5, mob + "," + facing + (i % 8 == 0 ? ",powered:1b" : ""));
        }
        i = 0;
        for (double x = MIN_X + 1; x < MAX_X; x += 2, i++) {
            summon(server, "spider", x, FLOOR, 22.5, mob + "," + facing);
            summon(server, "enderman", x, FLOOR, 24.5, mob + "," + facing
                    + (i % 2 == 0 ? ",carriedBlockState:\"minecraft:grass_block\"" : ""));
        }

        // Illagers and witches
        String[] illagers = {"pillager", "vindicator", "evoker", "witch"};
        String[] weapons = {"crossbow", "iron_axe", null, null};
        i = 0;
        for (double x = MIN_X + 0.5; x < MAX_X; x += 1.5, i++) {
            int k = i % illagers.length;
            String nbt = mob + "," + facing + (weapons[k] != null ? "," + equipment(null, weapons[k]) : "");
            if (i % 7 == 0) {
                nbt += ",CustomName:\"%s %d\",CustomNameVisible:1b".formatted(illagers[k], i);
            }
            summon(server, illagers[k], x, FLOOR, 26.5, nbt);
        }

        // Iron golems, horses (every other saddled), wolves and cats
        for (double x = MIN_X + 1.25; x < MAX_X; x += 2.5) {
            summon(server, "iron_golem", x, FLOOR, 28.5, mob + "," + facing);
        }
        i = 0;
        for (double x = MIN_X + 1; x < MAX_X; x += 2, i++) {
            String nbt = mob + "," + facing + ",Tame:1b,Variant:%d".formatted((i % 7) | ((i % 5) << 8));
            if (i % 2 == 0) {
                nbt += ",equipment:{saddle:{id:\"minecraft:saddle\",count:1}}";
            }
            summon(server, "horse", x, FLOOR, 30.5, nbt);
        }
        i = 0;
        for (double x = MIN_X + 0.5; x < MAX_X; x += 1.5, i++) {
            summon(server, i % 2 == 0 ? "wolf" : "cat", x, FLOOR, 32.5, mob + "," + facing);
        }

        // Minecarts and boats
        String[] vehicles = {"minecart", "chest_minecart", "oak_boat", "birch_chest_boat"};
        i = 0;
        for (double x = MIN_X + 1; x < MAX_X; x += 2, i++) {
            summon(server, vehicles[i % vehicles.length], x, FLOOR, 34.0, "Rotation:[90f,0f]");
        }

        // Text, item and block displays
        i = 0;
        for (int x = MIN_X; x < MAX_X; x++, i++) {
            switch (i % 3) {
                case 0 -> summon(server, "text_display", x + 0.5, FLOOR + 1.0, 35.5,
                        "text:\"Display %d\",billboard:\"center\"".formatted(i));
                case 1 -> summon(server, "item_display", x + 0.5, FLOOR + 0.5, 35.5,
                        "item:{id:\"minecraft:%s\",count:1},billboard:\"fixed\"".formatted(ITEMS[i % ITEMS.length]));
                default -> summon(server, "block_display", x + 0.25, FLOOR, 35.25,
                        "block_state:\"minecraft:%s\",transformation:[0.5f,0f,0f,0f,0f,0.5f,0f,0f,0f,0f,0.5f,0f,0f,0f,0f,1f]"
                                .formatted(DISPLAY_BLOCKS[i % DISPLAY_BLOCKS.length]));
            }
        }

        // A wall with item frames (every other one glowing) and paintings above them
        server.runCommand("fill %d %d %d %d %d %d minecraft:stone_bricks".formatted(MIN_X, FLOOR, WALL_Z, MAX_X - 1, FLOOR + 4, WALL_Z));
        for (int y = FLOOR; y < FLOOR + 4; y++) {
            for (int x = MIN_X; x < MAX_X; x++) {
                String item = ITEMS[Math.floorMod(x + y, ITEMS.length)];
                summon(server, (x + y) % 2 == 0 ? "item_frame" : "glow_item_frame", x + 0.5, y + 0.5, WALL_Z - 0.5,
                        "Facing:2b,Fixed:1b,Item:{id:\"minecraft:%s\",count:1}".formatted(item));
            }
        }
        for (int x = MIN_X; x < MAX_X; x++) {
            summon(server, "painting", x + 0.5, FLOOR + 4.5, WALL_Z - 0.5, "facing:2b,variant:\"minecraft:kebab\"");
        }
    }

    private static String helmet(int i) {
        return unbreakable(HELMETS[Math.floorMod(i, HELMETS.length)]);
    }

    private static String unbreakable(String item) {
        return "{id:\"minecraft:%s\",count:1,components:{\"minecraft:unbreakable\":{}}}".formatted(item);
    }

    private static String equipment(String head, String mainhand) {
        StringBuilder sb = new StringBuilder("equipment:{");
        if (head != null) {
            sb.append("head:").append(head);
        }
        if (mainhand != null) {
            if (head != null) {
                sb.append(',');
            }
            sb.append("mainhand:{id:\"minecraft:%s\",count:1}".formatted(mainhand));
        }
        return sb.append('}').toString();
    }

    private static void summon(TestServerContext server, String type, double x, double y, double z, String nbt) {
        server.runCommand("summon minecraft:%s %s %s %s {%s}".formatted(type, x, y, z, nbt));
    }

    private static Map<String, Integer> countEntities(Minecraft mc) {
        Map<String, Integer> counts = new TreeMap<>();
        for (Entity entity : mc.level.entitiesForRendering()) {
            counts.merge(BuiltInRegistries.ENTITY_TYPE.getKey(entity.getType()).getPath(), 1, Integer::sum);
        }
        return counts;
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
