package dev.ruthenium.trace;

import java.util.List;
import java.util.function.Function;
import net.fabricmc.fabric.api.client.gametest.v1.context.ClientGameTestContext;
import net.fabricmc.fabric.api.client.gametest.v1.context.TestServerContext;
import net.minecraft.client.KeyMapping;
import net.minecraft.client.Options;

/**
 * The scenarios. Coordinates: the flat world's grass is at y = -61, so the player stands at y =
 * -60. Yaw 0 faces +Z (south), -90 faces +X (east). Every scenario's area is first reset by {@link
 * #RESET}.
 */
public final class Scenarios {
    private Scenarios() {
    }

    /** Clears the test area and lays a stone floor over the flat world's dirt. */
    public static final List<String> RESET = List.of(
            "kill @e[type=!minecraft:player]",
            "fill -8 -60 -12 32 -45 12 minecraft:air",
            "fill -8 -62 -12 32 -62 12 minecraft:dirt",
            "fill -8 -61 -12 32 -61 12 minecraft:stone",
            "gamemode survival @a",
            "effect clear @a",
            "effect give @a minecraft:instant_health 1 10 true",
            "effect give @a minecraft:saturation 1 20 true",
            "clear @a");

    private static final Function<Options, KeyMapping> W = o -> o.keyUp;
    private static final Function<Options, KeyMapping> A = o -> o.keyLeft;
    private static final Function<Options, KeyMapping> S = o -> o.keyDown;
    private static final Function<Options, KeyMapping> D = o -> o.keyRight;
    private static final Function<Options, KeyMapping> JUMP = o -> o.keyJump;
    private static final Function<Options, KeyMapping> SNEAK = o -> o.keyShift;
    private static final Function<Options, KeyMapping> SPRINT = o -> o.keySprint;
    private static final Function<Options, KeyMapping> ATTACK = o -> o.keyAttack;
    private static final Function<Options, KeyMapping> USE = o -> o.keyUse;
    private static final Function<Options, KeyMapping> SWAP = o -> o.keySwapOffhand;

    private static Function<Options, KeyMapping> hotbar(int slot) {
        return o -> o.keyHotbarSlots[slot];
    }

    /** Holds the keys for {@code ticks} ticks, then releases them. */
    @SafeVarargs
    private static void hold(ClientGameTestContext ctx, int ticks, Function<Options, KeyMapping>... keys) {
        for (var key : keys) {
            ctx.getInput().holdKey(key);
        }
        ctx.waitTicks(ticks);
        for (var key : keys) {
            ctx.getInput().releaseKey(key);
        }
    }

    /** Mobs that stand still, stay, keep quiet and don't burn in daylight. */
    private static final String STILL = "{NoAI:1b,PersistenceRequired:1b,Silent:1b}";

    private static void fireproof(TestServerContext server) {
        server.runCommand("effect give @e[type=!minecraft:player] minecraft:fire_resistance infinite 0 true");
    }

    /** Presses jump twice within the 7-tick window, the second press held for {@code ticks}. */
    private static void doubleTapJump(ClientGameTestContext ctx, int ticks) {
        hold(ctx, 1, JUMP);
        ctx.waitTicks(2);
        hold(ctx, ticks, JUMP);
    }

    /**
     * Teleports the player to stand at (x, y, z) looking from its standing eyes at the point
     * (tx, ty, tz), then waits for the teleport to land (a teleport's rotation overrides a look
     * made in the same tick).
     */
    private static void aim(ClientGameTestContext ctx, TestServerContext server, double x, double y, double z,
            double tx, double ty, double tz) {
        double dx = tx - x;
        double dy = ty - (y + 1.62);
        double dz = tz - z;
        double yaw = Math.toDegrees(Math.atan2(-dx, dz));
        double pitch = Math.toDegrees(-Math.atan2(dy, Math.sqrt(dx * dx + dz * dz)));
        server.runCommand(String.format(java.util.Locale.ROOT, "tp @p %.4f %.4f %.4f %.4f %.4f", x, y, z, yaw, pitch));
        ctx.waitTicks(4);
    }

    /** One use click, then a pause past the use delay. */
    private static void use(ClientGameTestContext ctx) {
        ctx.getInput().pressKey(USE);
        ctx.waitTicks(5);
    }

    /** Selects a hotbar slot and waits for it to be sent. */
    private static void slot(ClientGameTestContext ctx, int slot) {
        ctx.getInput().pressKey(hotbar(slot));
        ctx.waitTicks(2);
    }

    /**
     * The breaking scenario's work area: the stone floor in front of the start mended, the
     * space above it cleared, {@code target} (if any) at 2 -60 0, and the player back at the
     * start looking at it.
     */
    private static void breakingArea(TestServerContext server, String target) {
        server.runCommand("fill 0 -61 -2 5 -61 2 minecraft:stone");
        server.runCommand("fill 0 -60 -2 5 -58 2 minecraft:air");
        if (target != null) {
            server.runCommand("setblock 2 -60 0 " + target);
        }
        server.runCommand("tp @p 0.5 -60 0.5 -90 30");
    }

    private static double[] start(double x, double y, double z, double yaw, double pitch) {
        return new double[] {x, y, z, yaw, pitch};
    }

    public static final List<Scenario> ALL = List.of(
            new Scenario("placing",
                    "Placing blocks: sneak-bridging backwards off a platform with use held, then walking off its end without sneaking; one click into the player (refused) and towering with jump and use held; stairs at the bottom and top of a face and two at a corner; a bottom slab doubled, a top slab; two fences that join and a stone that joins them; a torch on the floor and one on a wall; a door and its top half; snow on snow; stone over short grass; scaffolding climbing; a ladder; a block from the off hand past a stick; a block in creative (kept).",
                    List.of("item replace entity @a hotbar.0 with minecraft:stone 64",
                            "item replace entity @a hotbar.1 with minecraft:oak_stairs 16",
                            "item replace entity @a hotbar.2 with minecraft:oak_slab 16",
                            "item replace entity @a hotbar.3 with minecraft:oak_fence 16",
                            "item replace entity @a hotbar.4 with minecraft:torch 16",
                            "item replace entity @a hotbar.5 with minecraft:oak_door 4",
                            "item replace entity @a hotbar.6 with minecraft:scaffolding 16",
                            "item replace entity @a hotbar.7 with minecraft:snow 16",
                            "item replace entity @a hotbar.8 with minecraft:ladder 8",
                            "item replace entity @a weapon.offhand with minecraft:cobblestone 8",
                            "fill 0 -58 -1 2 -58 1 minecraft:stone",
                            "setblock 17 -60 3 minecraft:stone",
                            "setblock 17 -60 -6 minecraft:stone",
                            "setblock 23 -60 8 minecraft:stone",
                            "setblock 20 -60 -6 minecraft:stone",
                            "setblock 29 -60 -6 minecraft:short_grass"),
                    start(1.5, -57, 0.5, 90, 80), (ctx, server) -> {
                        // Sneak-bridging east, walking backwards and facing the platform.
                        ctx.waitTicks(10);
                        ctx.getInput().holdKey(SNEAK);
                        ctx.waitTicks(3);
                        hold(ctx, 50, S, USE);
                        ctx.getInput().releaseKey(SNEAK);
                        ctx.waitTicks(5);
                        // Off the bridge's end without sneaking, use still held.
                        hold(ctx, 12, S, USE);
                        ctx.waitTicks(25);
                        // Towering: a click into the player first, refused.
                        aim(ctx, server, 10.5, -60, 6.5, 10.5, -61, 6.5);
                        use(ctx);
                        hold(ctx, 60, JUMP, USE);
                        ctx.waitTicks(20);
                        // Stairs: on the floor, at the top of a face, and two meeting at a corner.
                        slot(ctx, 1);
                        aim(ctx, server, 14.5, -60, 0.5, 14.5, -60, 2.5);
                        use(ctx);
                        aim(ctx, server, 17.5, -60, 0.5, 17.5, -59.2, 3.0);
                        use(ctx);
                        aim(ctx, server, 20.5, -60, -0.5, 20.5, -60, 2.5);
                        use(ctx);
                        aim(ctx, server, 23.5, -60, 2.5, 21.5, -60, 2.5);
                        use(ctx);
                        // Slabs: a bottom slab doubled, then a top slab on a face's upper half.
                        slot(ctx, 2);
                        aim(ctx, server, 14.5, -60, -3.5, 14.5, -60, -5.5);
                        use(ctx);
                        aim(ctx, server, 14.5, -60, -3.5, 14.5, -59.5, -5.5);
                        use(ctx);
                        aim(ctx, server, 17.5, -60, -3.5, 17.5, -59.2, -5.0);
                        use(ctx);
                        // Fences that join, then a stone beside them.
                        slot(ctx, 3);
                        aim(ctx, server, 14.5, -60, 5.5, 14.5, -60, 8.5);
                        use(ctx);
                        aim(ctx, server, 15.5, -60, 5.5, 15.5, -60, 8.5);
                        use(ctx);
                        slot(ctx, 0);
                        aim(ctx, server, 16.5, -60, 5.5, 16.5, -60, 8.5);
                        use(ctx);
                        // Torches: on the floor, and on a wall.
                        slot(ctx, 4);
                        aim(ctx, server, 20.5, -60, 5.5, 20.5, -60, 7.5);
                        use(ctx);
                        aim(ctx, server, 23.5, -60, 5.5, 23.5, -59.5, 8.0);
                        use(ctx);
                        // A door, which puts its top half above it.
                        slot(ctx, 5);
                        aim(ctx, server, 26.5, -60, 0.5, 26.5, -60, 2.5);
                        use(ctx);
                        // Snow, then snow on the snow.
                        slot(ctx, 7);
                        aim(ctx, server, 26.5, -60, -3.5, 26.5, -60, -5.5);
                        use(ctx);
                        aim(ctx, server, 26.5, -60, -3.5, 26.5, -59.875, -5.5);
                        use(ctx);
                        // Stone over short grass.
                        slot(ctx, 0);
                        aim(ctx, server, 29.5, -60, -3.5, 29.5, -59.6, -5.5);
                        use(ctx);
                        // Scaffolding, then clicked twice on its side: it climbs.
                        slot(ctx, 6);
                        aim(ctx, server, 29.5, -60, 0.5, 29.5, -60, 2.5);
                        use(ctx);
                        aim(ctx, server, 29.5, -60, 0.5, 29.5, -59.5, 2.0);
                        use(ctx);
                        use(ctx);
                        // A ladder on a wall.
                        slot(ctx, 8);
                        aim(ctx, server, 20.5, -60, -3.5, 20.5, -59.5, -5.0);
                        use(ctx);
                        // The off hand's cobblestone, past a stick that places nothing.
                        server.runCommand("item replace entity @p hotbar.4 with minecraft:stick");
                        slot(ctx, 4);
                        ctx.waitTicks(3);
                        aim(ctx, server, 4.5, -60, 6.5, 4.5, -60, 8.5);
                        use(ctx);
                        // Creative: the stack stays whole.
                        server.runCommand("gamemode creative @p");
                        slot(ctx, 0);
                        ctx.waitTicks(3);
                        aim(ctx, server, 6.5, -60, 6.5, 6.5, -60, 8.5);
                        use(ctx);
                        server.runCommand("gamemode survival @p");
                        ctx.waitTicks(10);
                    }),
            new Scenario("scaffolding",
                    "Moving in scaffolding: walking into a column from the side (it has no sides); climbing it from inside with jump held until standing on its top; sneaking down through it to the ground; dropping onto a piece reaching out from the top, unsupported, whose thin floor holds the player even while sneaking; walking off it.",
                    List.of("fill 2 -60 3 2 -56 3 minecraft:scaffolding[distance=0,bottom=false]",
                            "setblock 2 -56 4 minecraft:scaffolding[distance=1,bottom=true]"),
                    start(2.5, -60, 0.5, 0, 0), (ctx, server) -> {
                        ctx.waitTicks(10);
                        hold(ctx, 14, W);
                        ctx.waitTicks(5);
                        // Inside the column: climb to its top and stand there.
                        server.runCommand("tp @p 2.5 -60 3.5 0 0");
                        ctx.waitTicks(5);
                        hold(ctx, 45, JUMP);
                        ctx.waitTicks(15);
                        // Sneaking lets the player down through the top.
                        hold(ctx, 50, SNEAK);
                        ctx.waitTicks(10);
                        // The unsupported piece: its thin floor holds the player, sneaking or not.
                        server.runCommand("tp @p 2.5 -55.5 4.5 0 0");
                        ctx.waitTicks(15);
                        hold(ctx, 15, SNEAK);
                        ctx.waitTicks(5);
                        hold(ctx, 12, W);
                        ctx.waitTicks(20);
                    }),
            new Scenario("using_blocks",
                    "Using blocks: a door opened, walked through and closed from the other side, an iron door that doesn't open by hand (stone in hand goes against it); a trapdoor in the floor opened under the player; a fence gate opened, walked through, closed and opened again from the other side (it turns); a lever twice, a button, a repeater's delay three times, a comparator, a note block, a daylight detector; a cake eaten when hungry; a candle on a cake; a dandelion potted; sweet berries picked; seeds in a composter; a sneaking click on the door with stone places it.",
                    List.of("setblock 2 -60 3 minecraft:oak_door[half=lower,facing=north]",
                            "setblock 2 -59 3 minecraft:oak_door[half=upper,facing=north]",
                            "setblock 4 -60 3 minecraft:iron_door[half=lower,facing=north]",
                            "setblock 4 -59 3 minecraft:iron_door[half=upper,facing=north]",
                            "setblock 6 -61 3 minecraft:oak_trapdoor[half=top,facing=north]",
                            "setblock 10 -60 3 minecraft:oak_fence_gate[facing=north]",
                            "setblock 14 -60 3 minecraft:lever[face=floor,facing=north]",
                            "setblock 16 -60 3 minecraft:stone_button[face=floor,facing=north]",
                            "setblock 18 -60 3 minecraft:repeater[facing=north]",
                            "setblock 20 -60 3 minecraft:comparator[facing=north]",
                            "setblock 22 -60 3 minecraft:note_block",
                            "setblock 24 -60 3 minecraft:daylight_detector",
                            "setblock 26 -60 3 minecraft:cake",
                            "setblock 28 -60 3 minecraft:cake",
                            "setblock 2 -60 -4 minecraft:flower_pot",
                            "setblock 4 -60 -4 minecraft:sweet_berry_bush[age=3]",
                            "setblock 6 -60 -4 minecraft:composter",
                            "item replace entity @a hotbar.1 with minecraft:candle 4",
                            "item replace entity @a hotbar.2 with minecraft:dandelion 4",
                            "item replace entity @a hotbar.3 with minecraft:wheat_seeds 16",
                            "item replace entity @a hotbar.4 with minecraft:stone 16"),
                    start(2.5, -60, 0.5, 0, 20), (ctx, server) -> {
                        ctx.waitTicks(10);
                        // The door: open it, walk through, turn round and close it.
                        aim(ctx, server, 2.5, -60, 0.5, 2.5, -59.5, 3.2);
                        use(ctx);
                        hold(ctx, 25, W);
                        ctx.waitTicks(5);
                        aim(ctx, server, 2.5, -60, 5.5, 2.1, -59.5, 3.5);
                        use(ctx);
                        // The iron door: nothing happens with an empty hand; with stone in
                        // hand the stone goes against it.
                        aim(ctx, server, 4.5, -60, 0.5, 4.5, -59.5, 3.9);
                        use(ctx);
                        slot(ctx, 4);
                        use(ctx);
                        slot(ctx, 0);
                        // The trapdoor in the floor, opened from on top of it: the player drops.
                        aim(ctx, server, 6.5, -60, 3.5, 6.5, -61, 3.5);
                        use(ctx);
                        ctx.waitTicks(15);
                        // The fence gate: opened, walked through, closed, opened from the far side.
                        aim(ctx, server, 10.5, -60, 0.5, 10.5, -59.5, 3.5);
                        use(ctx);
                        hold(ctx, 25, W);
                        ctx.waitTicks(5);
                        aim(ctx, server, 10.5, -60, 5.5, 10.5, -59.5, 3.5);
                        use(ctx);
                        use(ctx);
                        // Redstone parts.
                        aim(ctx, server, 14.5, -60, 1.5, 14.5, -59.9, 3.5);
                        use(ctx);
                        use(ctx);
                        aim(ctx, server, 16.5, -60, 1.5, 16.5, -59.95, 3.5);
                        use(ctx);
                        aim(ctx, server, 18.5, -60, 1.5, 18.5, -59.9, 3.5);
                        use(ctx);
                        use(ctx);
                        use(ctx);
                        aim(ctx, server, 20.5, -60, 1.5, 20.5, -59.9, 3.5);
                        use(ctx);
                        aim(ctx, server, 22.5, -60, 1.5, 22.5, -59.5, 3.0);
                        use(ctx);
                        aim(ctx, server, 24.5, -60, 1.5, 24.5, -59.7, 3.5);
                        use(ctx);
                        // A cake: refused at full hunger, eaten once hungry.
                        aim(ctx, server, 26.5, -60, 1.5, 26.5, -59.6, 3.5);
                        use(ctx);
                        server.runCommand("effect give @p minecraft:hunger 5 255 true");
                        ctx.waitTicks(110);
                        use(ctx);
                        // A candle on the other cake.
                        slot(ctx, 1);
                        aim(ctx, server, 28.5, -60, 1.5, 28.5, -59.6, 3.5);
                        use(ctx);
                        // A dandelion potted, berries picked, seeds composted.
                        slot(ctx, 2);
                        aim(ctx, server, 2.5, -60, -1.5, 2.5, -59.7, -3.5);
                        use(ctx);
                        slot(ctx, 0);
                        aim(ctx, server, 4.5, -60, -1.5, 4.5, -59.5, -3.5);
                        use(ctx);
                        slot(ctx, 3);
                        aim(ctx, server, 6.5, -60, -1.5, 6.5, -59.5, -3.5);
                        use(ctx);
                        use(ctx);
                        // Sneaking with stone in hand: the door isn't used, the stone is placed.
                        slot(ctx, 4);
                        aim(ctx, server, 2.5, -60, 0.5, 2.5, -59.5, 3.9);
                        ctx.getInput().holdKey(SNEAK);
                        ctx.waitTicks(3);
                        use(ctx);
                        ctx.getInput().releaseKey(SNEAK);
                        ctx.waitTicks(10);
                    }),
            new Scenario("tools_on_blocks",
                    "Items used on blocks: a shovel making a path the player then walks onto, a hoe making farmland, an axe stripping a log, flint and steel lighting the floor, bone meal on a grass block, honeycomb on copper, a water bottle on dirt, a lily pad on water, powder snow from its bucket.",
                    List.of("fill 0 -61 2 12 -61 4 minecraft:grass_block",
                            "setblock 6 -60 3 minecraft:oak_log",
                            "setblock 10 -60 3 minecraft:copper_block",
                            "setblock 14 -61 3 minecraft:dirt",
                            "setblock 18 -61 3 minecraft:water",
                            "item replace entity @a hotbar.0 with minecraft:iron_shovel",
                            "item replace entity @a hotbar.1 with minecraft:iron_hoe",
                            "item replace entity @a hotbar.2 with minecraft:iron_axe",
                            "item replace entity @a hotbar.3 with minecraft:flint_and_steel",
                            "item replace entity @a hotbar.4 with minecraft:bone_meal 8",
                            "item replace entity @a hotbar.5 with minecraft:honeycomb 4",
                            "item replace entity @a hotbar.6 with minecraft:potion[potion_contents={potion:\"minecraft:water\"}]",
                            "item replace entity @a hotbar.7 with minecraft:lily_pad 4",
                            "item replace entity @a hotbar.8 with minecraft:powder_snow_bucket"),
                    start(2.5, -60, 0.5, 0, 40), (ctx, server) -> {
                        ctx.waitTicks(10);
                        // A path, walked onto (a sixteenth lower).
                        aim(ctx, server, 2.5, -60, 0.5, 2.5, -60, 2.5);
                        use(ctx);
                        hold(ctx, 15, W);
                        ctx.waitTicks(5);
                        slot(ctx, 1);
                        aim(ctx, server, 4.5, -60, 0.5, 4.5, -60, 2.5);
                        use(ctx);
                        slot(ctx, 2);
                        aim(ctx, server, 6.5, -60, 0.5, 6.5, -59.5, 3.0);
                        use(ctx);
                        slot(ctx, 3);
                        aim(ctx, server, 8.5, -60, 0.5, 8.5, -60, 2.5);
                        use(ctx);
                        slot(ctx, 4);
                        aim(ctx, server, 12.5, -60, 0.5, 12.5, -60, 3.5);
                        use(ctx);
                        slot(ctx, 5);
                        aim(ctx, server, 10.5, -60, 0.5, 10.5, -59.5, 3.0);
                        use(ctx);
                        slot(ctx, 6);
                        aim(ctx, server, 14.5, -60, 0.5, 14.5, -60, 3.5);
                        use(ctx);
                        slot(ctx, 7);
                        aim(ctx, server, 18.5, -60, 0.5, 18.5, -60.1, 3.5);
                        use(ctx);
                        slot(ctx, 8);
                        aim(ctx, server, 22.5, -60, 0.5, 22.5, -60, 2.5);
                        use(ctx);
                        ctx.waitTicks(10);
                    }),
            new Scenario("walk_directions",
                    "Walk 15 ticks in each of the eight key directions on stone, 5 idle ticks between.",
                    List.of(), start(0.5, -60, 0.5, 0, 0), (ctx, server) -> {
                        List<List<Function<Options, KeyMapping>>> directions = List.of(
                                List.of(W), List.of(W, D), List.of(D), List.of(S, D),
                                List.of(S), List.of(S, A), List.of(A), List.of(W, A));
                        for (var keys : directions) {
                            for (var key : keys) {
                                ctx.getInput().holdKey(key);
                            }
                            ctx.waitTicks(15);
                            for (var key : keys) {
                                ctx.getInput().releaseKey(key);
                            }
                            ctx.waitTicks(5);
                        }
                    }),
            new Scenario("sprint_jump",
                    "Hold forward, sprint and jump for 40 ticks (sprint-jump chain), then stop.",
                    List.of(), start(0.5, -60, 0.5, -90, 0), (ctx, server) -> {
                        hold(ctx, 40, W, SPRINT, JUMP);
                        ctx.waitTicks(20);
                    }),
            new Scenario("sneak_edge",
                    "On a 5x5 platform 3 blocks high: sneak forward into the edge, sneak sideways along it, then walk off without sneaking.",
                    List.of("fill 0 -60 -2 4 -58 2 minecraft:stone"), start(2.5, -57, 0.5, -90, 0), (ctx, server) -> {
                        hold(ctx, 30, W, SNEAK);
                        ctx.waitTicks(5);
                        hold(ctx, 20, D, SNEAK);
                        ctx.waitTicks(5);
                        hold(ctx, 15, W);
                        ctx.waitTicks(30);
                    }),
            new Scenario("ice",
                    "Sprint from ice onto blue ice, then let go and slide.",
                    List.of("fill 0 -61 -1 9 -61 1 minecraft:ice", "fill 10 -61 -1 19 -61 1 minecraft:blue_ice"),
                    start(0.5, -60, 0.5, -90, 0), (ctx, server) -> {
                        hold(ctx, 20, W, SPRINT);
                        ctx.waitTicks(60);
                    }),
            new Scenario("slime_bounce",
                    "Walk off a 10-block pillar onto slime, bounce until settled, then jump on the slime.",
                    List.of("fill 1 -61 -2 6 -61 2 minecraft:slime_block", "fill 0 -60 0 0 -51 0 minecraft:stone"),
                    start(0.5, -50, 0.5, -90, 0), (ctx, server) -> {
                        hold(ctx, 6, W);
                        ctx.waitTicks(80);
                        hold(ctx, 20, JUMP);
                        ctx.waitTicks(20);
                    }),
            new Scenario("ladder",
                    "Climb a ladder by walking into it, hold on by sneaking, then let go and slide down.",
                    List.of("fill 2 -60 -1 2 -52 1 minecraft:stone", "fill 1 -60 0 1 -53 0 minecraft:ladder[facing=west]"),
                    start(0.5, -60, 0.5, -90, 0), (ctx, server) -> {
                        hold(ctx, 40, W);
                        hold(ctx, 20, W, SNEAK);
                        hold(ctx, 20, SNEAK);
                        ctx.waitTicks(40);
                    }),
            new Scenario("swim",
                    "Walk into a 2-deep pool, sprint-swim, then swim up.",
                    List.of("fill 0 -62 -2 12 -61 2 minecraft:water"), start(-1.5, -60, 0.5, -90, 0), (ctx, server) -> {
                        hold(ctx, 15, W);
                        hold(ctx, 40, W, SPRINT);
                        hold(ctx, 20, W, JUMP);
                        ctx.waitTicks(20);
                    }),
            new Scenario("water_current",
                    "In a walled 3-wide channel of settled flowing water (sources at x = 0, levels 1 to 7 downstream): walk upstream against the current, jump in the sources, drift, sneak downstream, strafe, jump in the shallow end.",
                    List.of("fill -1 -60 -2 9 -59 2 minecraft:stone", "fill 0 -60 -1 8 -59 1 minecraft:air",
                            "fill 0 -60 -1 0 -60 1 minecraft:water",
                            "fill 1 -60 -1 1 -60 1 minecraft:water[level=1]",
                            "fill 2 -60 -1 2 -60 1 minecraft:water[level=2]",
                            "fill 3 -60 -1 3 -60 1 minecraft:water[level=3]",
                            "fill 4 -60 -1 4 -60 1 minecraft:water[level=4]",
                            "fill 5 -60 -1 5 -60 1 minecraft:water[level=5]",
                            "fill 6 -60 -1 6 -60 1 minecraft:water[level=6]",
                            "fill 7 -60 -1 7 -60 1 minecraft:water[level=7]"),
                    start(8.5, -60, 0.5, 90, 0), (ctx, server) -> {
                        hold(ctx, 40, W);
                        hold(ctx, 10, JUMP);
                        ctx.waitTicks(20);
                        ctx.getInput().lookAt(-90, 0);
                        hold(ctx, 15, W, SNEAK);
                        hold(ctx, 10, A);
                        hold(ctx, 15, W, JUMP);
                        ctx.waitTicks(20);
                    }),
            new Scenario("waterfall",
                    "Drop into a 1x1 shaft of falling water fed by a source at the top, sink with the current, swim up, then swim-sprint looking up.",
                    List.of("fill 3 -60 -1 5 -53 1 minecraft:stone", "fill 5 -52 -1 5 -51 1 minecraft:stone",
                            "fill 4 -60 0 4 -53 0 minecraft:air", "fill 4 -60 0 4 -55 0 minecraft:water[level=8]",
                            "setblock 4 -54 0 minecraft:water"),
                    start(3.5, -52, 0.5, -90, 0), (ctx, server) -> {
                        hold(ctx, 15, W);
                        ctx.waitTicks(25);
                        hold(ctx, 40, JUMP);
                        ctx.waitTicks(30);
                        ctx.getInput().lookAt(-90, -60);
                        hold(ctx, 30, W, SPRINT);
                        ctx.waitTicks(20);
                    }),
            new Scenario("lava",
                    "With fire resistance: walk into a 2-deep lava pool, sink, swim across holding jump and hop out over the far edge.",
                    List.of("effect give @a minecraft:fire_resistance 300 0 true", "fill 0 -62 -2 6 -61 2 minecraft:lava"),
                    start(-1.5, -60, 0.5, -90, 0), (ctx, server) -> {
                        hold(ctx, 30, W);
                        hold(ctx, 50, W, JUMP);
                        ctx.waitTicks(20);
                    }),
            new Scenario("step_up",
                    "Walk over a slab, a carpet, a 3-layer snow block and stairs (stepping up), then jump onto a full block.",
                    List.of("setblock 2 -60 0 minecraft:smooth_stone_slab", "setblock 4 -60 0 minecraft:white_carpet",
                            "setblock 6 -60 0 minecraft:snow[layers=3]", "setblock 8 -60 0 minecraft:oak_stairs[facing=east]",
                            "setblock 10 -60 0 minecraft:stone"),
                    start(0.5, -60, 0.5, -90, 0), (ctx, server) -> {
                        hold(ctx, 50, W);
                        hold(ctx, 15, W, JUMP);
                        ctx.waitTicks(10);
                    }),
            new Scenario("head_hitter",
                    "Sprint-jump under a ceiling 2 blocks above the floor.",
                    List.of("fill 0 -58 -1 20 -58 1 minecraft:stone"), start(0.5, -60, 0.5, -90, 0), (ctx, server) -> {
                        hold(ctx, 40, W, SPRINT, JUMP);
                        ctx.waitTicks(10);
                    }),
            new Scenario("mouse_turn",
                    "Walk forward while the mouse turns the camera every tick (turns are applied per frame).",
                    List.of(), start(0.5, -60, 0.5, 0, 0), (ctx, server) -> {
                        ctx.getInput().holdKey(W);
                        for (int i = 0; i < 40; i++) {
                            ctx.getInput().moveCursor(25, 0);
                            ctx.waitTick();
                        }
                        for (int i = 0; i < 20; i++) {
                            ctx.getInput().moveCursor(-40, 6);
                            ctx.waitTick();
                        }
                        ctx.getInput().releaseKey(W);
                        ctx.waitTicks(5);
                    }),
            new Scenario("combat_basics",
                    "Hotbar keys and scrolling, attacking an armor stand, a missed swing, two quick attacks and blocking with a shield.",
                    List.of("item replace entity @a hotbar.0 with minecraft:diamond_sword",
                            "item replace entity @a hotbar.1 with minecraft:shield"),
                    start(0.5, -60, 0.5, -90, 0), (ctx, server) -> {
                        server.runCommand("summon minecraft:armor_stand 2.5 -60 0.5");
                        ctx.getInput().pressKey(hotbar(0));
                        ctx.waitTicks(5);
                        ctx.getInput().pressKey(ATTACK);
                        ctx.waitTicks(25);
                        ctx.getInput().lookAt(0, 0);
                        ctx.getInput().pressKey(ATTACK);
                        ctx.waitTicks(10);
                        ctx.getInput().lookAt(-90, 0);
                        ctx.getInput().pressKey(hotbar(1));
                        hold(ctx, 20, USE);
                        ctx.waitTicks(5);
                        ctx.getInput().scroll(1);
                        ctx.waitTicks(5);
                        ctx.getInput().pressKey(ATTACK);
                        ctx.getInput().pressKey(ATTACK);
                        ctx.waitTicks(20);
                    }),
            new Scenario("attacking",
                    "Sprinting hits that slow the attacker: a fist on a boat with sprint held (the sprint resumes) and a weaker hit after it; a sword on a minecart after a double-tapped sprint (the sprint ends); a sword on a zombie (the client doesn't slow; the server ends the sprint); two clicks at the sky in one tick and the attack key held there; a spear jab refused before it is charged, then made.",
                    List.of("difficulty normal",
                            "item replace entity @a hotbar.1 with minecraft:diamond_sword",
                            "item replace entity @a hotbar.2 with minecraft:wooden_spear"),
                    start(0.5, -60, 0.5, -90, 30), (ctx, server) -> {
                        server.runCommand("summon minecraft:oak_boat 3.0 -60 0.5");
                        ctx.waitTicks(10);
                        ctx.getInput().holdKey(W);
                        ctx.getInput().holdKey(SPRINT);
                        ctx.waitTicks(2);
                        ctx.getInput().pressKey(ATTACK);
                        ctx.waitTicks(3);
                        ctx.getInput().pressKey(ATTACK);
                        ctx.waitTicks(6);
                        ctx.getInput().releaseKey(W);
                        ctx.getInput().releaseKey(SPRINT);
                        ctx.waitTicks(10);
                        server.runCommand("kill @e[type=minecraft:oak_boat]");
                        server.runCommand("kill @e[type=minecraft:item]");
                        server.runCommand("tp @p 0.5 -60 0.5 -90 30");
                        ctx.getInput().pressKey(hotbar(1));
                        server.runCommand("summon minecraft:minecart 3.0 -60 0.5");
                        ctx.waitTicks(20);
                        hold(ctx, 1, W);
                        ctx.waitTicks(2);
                        ctx.getInput().holdKey(W);
                        ctx.waitTicks(3);
                        ctx.getInput().pressKey(ATTACK);
                        ctx.waitTicks(8);
                        ctx.getInput().releaseKey(W);
                        ctx.waitTicks(10);
                        server.runCommand("kill @e[type=minecraft:minecart]");
                        server.runCommand("kill @e[type=minecraft:item]");
                        server.runCommand("tp @p 0.5 -60 0.5 -90 30");
                        server.runCommand("summon minecraft:zombie 3.0 -60 0.5 " + STILL);
                        fireproof(server);
                        ctx.waitTicks(15);
                        ctx.getInput().holdKey(W);
                        ctx.getInput().holdKey(SPRINT);
                        ctx.waitTicks(2);
                        ctx.getInput().pressKey(ATTACK);
                        ctx.waitTicks(8);
                        ctx.getInput().releaseKey(W);
                        ctx.getInput().releaseKey(SPRINT);
                        // Dying mobs push for 20 ticks.
                        server.runCommand("kill @e[type=minecraft:zombie]");
                        ctx.waitTicks(30);
                        server.runCommand("kill @e[type=minecraft:item]");
                        ctx.getInput().lookAt(-90, -60);
                        ctx.waitTicks(15);
                        ctx.getInput().pressKey(ATTACK);
                        ctx.getInput().pressKey(ATTACK);
                        ctx.waitTicks(5);
                        hold(ctx, 15, ATTACK);
                        ctx.waitTicks(5);
                        ctx.getInput().pressKey(hotbar(2));
                        ctx.waitTicks(2);
                        ctx.getInput().pressKey(ATTACK);
                        ctx.waitTicks(15);
                        ctx.getInput().pressKey(ATTACK);
                        ctx.waitTicks(10);
                    }),
            new Scenario("breaking",
                    "Breaking blocks: dirt by hand (finished, then the floor behind started and given up); stone with a wooden pickaxe, its face changed half way, then two floor blocks in turn; a torch broken at once, twice, and once while digging the floor; the block under the player (it falls, and digs on in the air); haste II, mining fatigue, digging while jumping; a stone pickaxe with efficiency III; iron ore with a wooden pickaxe; a waterlogged pane (water left behind); redstone ore lit by the first hit; creative breaking every sixth tick, a waterlogged pane there, and a sword that breaks nothing; adventure mode with a pickaxe that may break only stone.",
                    List.of("gamerule block_drops false",
                            "item replace entity @a hotbar.1 with minecraft:wooden_pickaxe",
                            "item replace entity @a hotbar.2 with minecraft:stone_pickaxe[enchantments={efficiency:3}]",
                            "item replace entity @a hotbar.3 with minecraft:diamond_sword",
                            "setblock 2 -60 0 minecraft:dirt"),
                    start(0.5, -60, 0.5, -90, 30), (ctx, server) -> {
                        // Dirt by hand: 15 ticks; then the floor behind, given up.
                        ctx.waitTicks(10);
                        hold(ctx, 24, ATTACK);
                        ctx.waitTicks(5);
                        // Stone with a wooden pickaxe: the west face, then the top face of the
                        // same block, finished; the floor beside the player, then the floor
                        // behind the stone (the first given up for it), let go.
                        breakingArea(server, "minecraft:stone");
                        ctx.getInput().pressKey(hotbar(1));
                        ctx.waitTicks(10);
                        ctx.getInput().holdKey(ATTACK);
                        ctx.waitTicks(6);
                        ctx.getInput().lookAt(-90, 20);
                        ctx.waitTicks(25);
                        ctx.getInput().lookAt(-90, 50);
                        ctx.waitTicks(12);
                        ctx.getInput().lookAt(-90, 30);
                        ctx.waitTicks(6);
                        ctx.getInput().releaseKey(ATTACK);
                        ctx.waitTicks(5);
                        // A torch breaks at once: a click, then the key held (the floor behind
                        // is started at once, and given up).
                        breakingArea(server, "minecraft:torch");
                        ctx.getInput().pressKey(hotbar(0));
                        ctx.waitTicks(10);
                        ctx.getInput().pressKey(ATTACK);
                        ctx.waitTicks(5);
                        server.runCommand("setblock 2 -60 0 minecraft:torch");
                        ctx.waitTicks(5);
                        hold(ctx, 6, ATTACK);
                        ctx.waitTicks(5);
                        // Digging the floor, then a click on a torch with the key still held: the
                        // floor given up, the torch broken at once, and breaking stopped in the
                        // same tick (the floor given up a second time).
                        breakingArea(server, "minecraft:torch");
                        ctx.waitTicks(5);
                        ctx.getInput().lookAt(-90, 50);
                        ctx.waitTicks(5);
                        ctx.getInput().holdKey(ATTACK);
                        ctx.waitTicks(4);
                        ctx.getInput().releaseKey(ATTACK);
                        ctx.getInput().lookAt(-90, 30);
                        ctx.getInput().holdKey(ATTACK);
                        ctx.waitTicks(3);
                        ctx.getInput().releaseKey(ATTACK);
                        ctx.waitTicks(5);
                        // The block under the player: it falls, and digs the dirt below in the air.
                        breakingArea(server, null);
                        server.runCommand("tp @p 0.5 -60 0.5 -90 90");
                        ctx.getInput().pressKey(hotbar(1));
                        ctx.waitTicks(10);
                        hold(ctx, 30, ATTACK);
                        ctx.waitTicks(10);
                        // Haste II, then mining fatigue (given up), then digging while jumping.
                        breakingArea(server, "minecraft:dirt");
                        ctx.getInput().pressKey(hotbar(0));
                        server.runCommand("effect give @p minecraft:haste 60 1 true");
                        ctx.waitTicks(10);
                        hold(ctx, 14, ATTACK);
                        ctx.waitTicks(3);
                        server.runCommand("effect clear @p minecraft:haste");
                        server.runCommand("effect give @p minecraft:mining_fatigue 60 0 true");
                        server.runCommand("setblock 2 -60 0 minecraft:dirt");
                        ctx.waitTicks(5);
                        hold(ctx, 10, ATTACK);
                        server.runCommand("effect clear @p minecraft:mining_fatigue");
                        ctx.waitTicks(5);
                        ctx.getInput().holdKey(JUMP);
                        hold(ctx, 30, ATTACK);
                        ctx.getInput().releaseKey(JUMP);
                        ctx.waitTicks(10);
                        // Efficiency III on a stone pickaxe: 4 ticks.
                        breakingArea(server, "minecraft:stone");
                        ctx.getInput().pressKey(hotbar(2));
                        ctx.waitTicks(10);
                        hold(ctx, 8, ATTACK);
                        ctx.waitTicks(3);
                        // Iron ore with a wooden pickaxe: the pickaxe's speed, but the wrong tier
                        // (a hundredth, not a thirtieth, per tick); given up.
                        breakingArea(server, "minecraft:iron_ore");
                        ctx.getInput().pressKey(hotbar(1));
                        ctx.waitTicks(10);
                        hold(ctx, 6, ATTACK);
                        ctx.waitTicks(3);
                        // A waterlogged pane leaves its water.
                        breakingArea(server, "minecraft:glass_pane[waterlogged=true]");
                        ctx.getInput().pressKey(hotbar(0));
                        ctx.waitTicks(10);
                        hold(ctx, 11, ATTACK);
                        server.runCommand("fill 0 -60 -2 5 -60 2 minecraft:air");
                        ctx.waitTicks(10);
                        // Redstone ore lights up at the first hit.
                        breakingArea(server, "minecraft:redstone_ore");
                        ctx.waitTicks(10);
                        hold(ctx, 4, ATTACK);
                        ctx.waitTicks(5);
                        // Creative: the dirt at once, then the floor behind every sixth tick;
                        // a sword breaks nothing.
                        breakingArea(server, "minecraft:dirt");
                        server.runCommand("gamemode creative @p");
                        ctx.waitTicks(10);
                        hold(ctx, 16, ATTACK);
                        ctx.waitTicks(5);
                        // A waterlogged pane: its water isn't air, so holding the key goes on
                        // in the same tick.
                        breakingArea(server, "minecraft:glass_pane[waterlogged=true]");
                        ctx.waitTicks(10);
                        hold(ctx, 3, ATTACK);
                        server.runCommand("fill 0 -60 -2 5 -60 2 minecraft:air");
                        ctx.waitTicks(5);
                        breakingArea(server, "minecraft:dirt");
                        ctx.getInput().pressKey(hotbar(3));
                        ctx.waitTicks(10);
                        hold(ctx, 13, ATTACK);
                        server.runCommand("gamemode survival @p");
                        ctx.waitTicks(5);
                        // Adventure mode: a pickaxe that may break only stone. Not the dirt
                        // (only swings), then the stone floor, then not the dirt under it.
                        breakingArea(server, "minecraft:dirt");
                        server.runCommand("gamemode adventure @p");
                        server.runCommand("item replace entity @p hotbar.4 with minecraft:wooden_pickaxe[can_break={blocks:\"minecraft:stone\"}]");
                        ctx.getInput().pressKey(hotbar(4));
                        ctx.waitTicks(10);
                        hold(ctx, 5, ATTACK);
                        ctx.getInput().lookAt(-90, 50);
                        ctx.waitTicks(2);
                        hold(ctx, 32, ATTACK);
                        server.runCommand("gamemode survival @p");
                        server.runCommand("gamerule block_drops true");
                        ctx.waitTicks(5);
                    }),
            new Scenario("entity_push",
                    "Mobs push the player: a zombie summoned overlapping it, walking into it and standing in it; a baby zombie to the side; a size-2 slime behind.",
                    List.of("difficulty normal"), start(0.5, -60, 0.5, -90, 0), (ctx, server) -> {
                        server.runCommand("summon minecraft:zombie 0.8 -60 0.6 " + STILL);
                        fireproof(server);
                        ctx.waitTicks(15);
                        hold(ctx, 20, W);
                        ctx.waitTicks(20);
                        server.runCommand("execute at @p run summon minecraft:zombie ~0.2 ~ ~0.5 {NoAI:1b,PersistenceRequired:1b,Silent:1b,IsBaby:1b}");
                        fireproof(server);
                        ctx.waitTicks(20);
                        hold(ctx, 10, D);
                        ctx.waitTicks(10);
                        server.runCommand("execute at @p run summon minecraft:slime ~-0.5 ~ ~-0.3 {NoAI:1b,PersistenceRequired:1b,Silent:1b,Size:1}");
                        ctx.waitTicks(20);
                        hold(ctx, 15, S);
                        ctx.waitTicks(20);
                        // Gone before the next scenario: a size-1 slime doesn't split, and
                        // dying mobs push for the 20 ticks they take to go.
                        server.runCommand("data merge entity @e[type=minecraft:slime,limit=1] {Size:0}");
                        server.runCommand("kill @e[type=!minecraft:player]");
                        ctx.waitTicks(30);
                    }),
            new Scenario("entity_moving",
                    "A zombie moved 0.15 blocks a tick through the standing player (interpolated pushes), then a second one sliding sideways, then both killed.",
                    List.of("difficulty normal"), start(0.5, -60, 0.5, -90, 0), (ctx, server) -> {
                        server.runCommand("summon minecraft:zombie 3.5 -60 0.6 " + STILL);
                        fireproof(server);
                        ctx.waitTicks(10);
                        for (int i = 0; i < 40; i++) {
                            server.runCommand("execute as @e[type=minecraft:zombie] at @s run tp @s ~-0.15 ~ ~");
                            ctx.waitTick();
                        }
                        ctx.waitTicks(10);
                        server.runCommand("execute at @p run summon minecraft:zombie ~0.3 ~ ~2 " + STILL);
                        fireproof(server);
                        ctx.waitTicks(5);
                        for (int i = 0; i < 30; i++) {
                            server.runCommand("execute as @e[type=minecraft:zombie,sort=nearest,limit=1] at @s run tp @s ~ ~ ~-0.12");
                            ctx.waitTick();
                        }
                        ctx.waitTicks(10);
                        server.runCommand("kill @e[type=minecraft:zombie]");
                        ctx.waitTicks(30);
                    }),
            new Scenario("entity_rules",
                    "Team collision rules decide who pushes (never, pushOtherTeams with the zombie alone and with both on the team, pushOwnTeam), then a boat pushes the player.",
                    List.of("difficulty normal"), start(0.5, -60, 0.5, -90, 0), (ctx, server) -> {
                        server.runCommand("team add ruthenium_rules");
                        server.runCommand("summon minecraft:zombie 0.8 -60 0.6 " + STILL);
                        fireproof(server);
                        server.runCommand("team modify ruthenium_rules collisionRule never");
                        server.runCommand("team join ruthenium_rules @e[type=minecraft:zombie]");
                        ctx.waitTicks(15);
                        server.runCommand("team modify ruthenium_rules collisionRule pushOtherTeams");
                        ctx.waitTicks(15);
                        server.runCommand("team join ruthenium_rules @p");
                        ctx.waitTicks(15);
                        server.runCommand("team modify ruthenium_rules collisionRule pushOwnTeam");
                        ctx.waitTicks(15);
                        server.runCommand("team leave @p");
                        ctx.waitTicks(15);
                        server.runCommand("team remove ruthenium_rules");
                        server.runCommand("kill @e[type=minecraft:zombie]");
                        ctx.waitTicks(30);
                        server.runCommand("execute at @p run summon minecraft:oak_boat ~1.2 ~ ~0.2");
                        ctx.waitTicks(10);
                        hold(ctx, 15, W);
                        ctx.waitTicks(20);
                        server.runCommand("kill @e[type=minecraft:oak_boat]");
                        ctx.waitTicks(5);
                    }),
            new Scenario("flying",
                    "Creative flight: double-tap jump to take off and rise, fly forward, sprint-fly, strafe, sink with sneak, hover; double-tap to drop, take off again and land, which ends the flight.",
                    List.of(), start(0.5, -60, 0.5, -90, 0), (ctx, server) -> {
                        server.runCommand("gamemode creative @a");
                        ctx.waitTicks(5);
                        doubleTapJump(ctx, 10);
                        hold(ctx, 10, W);
                        hold(ctx, 10, W, SPRINT);
                        ctx.waitTicks(5);
                        hold(ctx, 10, A);
                        hold(ctx, 6, SNEAK);
                        ctx.waitTicks(10);
                        doubleTapJump(ctx, 1);
                        ctx.waitTicks(30);
                        doubleTapJump(ctx, 6);
                        hold(ctx, 30, SNEAK);
                        ctx.waitTicks(10);
                        server.runCommand("gamemode survival @a");
                        ctx.waitTicks(5);
                    }),
            new Scenario("spectator",
                    "Spectator mode: sink through the floor and come back up, fly through a wall, sprint-fly, rise, then back to survival in the air and fall.",
                    List.of("fill 4 -60 -3 4 -57 3 minecraft:stone"), start(0.5, -60, 0.5, -90, 0), (ctx, server) -> {
                        server.runCommand("gamemode spectator @a");
                        ctx.waitTicks(5);
                        hold(ctx, 8, SNEAK);
                        hold(ctx, 12, JUMP);
                        hold(ctx, 15, W);
                        hold(ctx, 10, W, SPRINT);
                        ctx.waitTicks(10);
                        hold(ctx, 8, JUMP);
                        server.runCommand("gamemode survival @a");
                        ctx.waitTicks(30);
                    }),
            new Scenario("effects",
                    "Effects that move the player: jumps with jump boost II, levitation V, slow falling back down while walking, speed III, blindness ending a sprint and stopping a new one, then dolphin's grace sprint-swimming across a pool.",
                    List.of("fill 22 -62 -3 30 -61 3 minecraft:water"), start(0.5, -60, 0.5, -90, 0), (ctx, server) -> {
                        server.runCommand("effect give @a minecraft:jump_boost 60 1");
                        ctx.waitTicks(3);
                        hold(ctx, 25, JUMP);
                        ctx.waitTicks(15);
                        server.runCommand("effect clear @a minecraft:jump_boost");
                        server.runCommand("effect give @a minecraft:levitation 60 4");
                        ctx.waitTicks(30);
                        server.runCommand("effect clear @a minecraft:levitation");
                        server.runCommand("effect give @a minecraft:slow_falling 60 0");
                        hold(ctx, 10, W);
                        ctx.waitTicks(35);
                        server.runCommand("effect clear @a minecraft:slow_falling");
                        server.runCommand("effect give @a minecraft:speed 60 2");
                        ctx.waitTicks(3);
                        hold(ctx, 15, W);
                        hold(ctx, 10, W, SPRINT);
                        server.runCommand("effect clear @a minecraft:speed");
                        ctx.waitTicks(10);
                        ctx.getInput().holdKey(W);
                        ctx.getInput().holdKey(SPRINT);
                        ctx.waitTicks(10);
                        server.runCommand("effect give @a minecraft:blindness 60 0");
                        ctx.waitTicks(10);
                        ctx.getInput().releaseKey(W);
                        ctx.getInput().releaseKey(SPRINT);
                        ctx.waitTicks(5);
                        hold(ctx, 10, W, SPRINT);
                        server.runCommand("effect clear @a minecraft:blindness");
                        server.runCommand("effect give @a minecraft:dolphins_grace 60 0");
                        ctx.waitTicks(5);
                        hold(ctx, 30, W, SPRINT);
                        ctx.waitTicks(10);
                        server.runCommand("effect clear @a");
                        ctx.waitTicks(5);
                    }),
            new Scenario("eating",
                    "Eating and drinking while walking: cooked beef refused at full hunger, then eaten after hunger drains it, eaten while sprinting and stopped half way; a golden apple, a water potion, a honey bottle and milk to the end; a golden apple looking at the floor.",
                    List.of(), start(0.5, -60, 0.5, -90, 0), (ctx, server) -> {
                        server.runCommand("item replace entity @a hotbar.0 with minecraft:cooked_beef 4");
                        server.runCommand("item replace entity @a hotbar.1 with minecraft:golden_apple 4");
                        server.runCommand("item replace entity @a hotbar.2 with minecraft:potion[minecraft:potion_contents={potion:\"minecraft:water\"}]");
                        server.runCommand("item replace entity @a hotbar.3 with minecraft:honey_bottle 2");
                        server.runCommand("item replace entity @a hotbar.4 with minecraft:milk_bucket");
                        ctx.waitTicks(5);
                        hold(ctx, 15, W, USE);
                        ctx.waitTicks(5);
                        server.runCommand("effect give @a minecraft:hunger 4 255 true");
                        ctx.waitTicks(85);
                        hold(ctx, 45, W, USE);
                        ctx.getInput().holdKey(W);
                        ctx.getInput().holdKey(SPRINT);
                        ctx.waitTicks(10);
                        hold(ctx, 15, USE);
                        ctx.waitTicks(5);
                        ctx.getInput().releaseKey(W);
                        ctx.getInput().releaseKey(SPRINT);
                        ctx.getInput().lookAt(90, 0);
                        ctx.getInput().pressKey(hotbar(1));
                        ctx.waitTicks(2);
                        hold(ctx, 40, W, USE);
                        ctx.getInput().pressKey(hotbar(2));
                        ctx.waitTicks(2);
                        hold(ctx, 40, S, USE);
                        ctx.getInput().pressKey(hotbar(3));
                        ctx.waitTicks(2);
                        hold(ctx, 45, W, USE);
                        ctx.getInput().pressKey(hotbar(4));
                        ctx.waitTicks(2);
                        hold(ctx, 36, S, USE);
                        ctx.getInput().pressKey(hotbar(1));
                        ctx.getInput().lookAt(-90, 70);
                        ctx.waitTicks(2);
                        hold(ctx, 12, W, USE);
                        ctx.waitTicks(10);
                    }),
            new Scenario("offhand",
                    "The off hand: a golden apple eaten from it with an empty main hand, past a sword and past a bow without arrows, kept on through a hotbar change; a shield blocking from it while walking; swapping hands while blocking and while eating.",
                    List.of(), start(0.5, -60, 0.5, -90, 0), (ctx, server) -> {
                        server.runCommand("item replace entity @a weapon.offhand with minecraft:golden_apple 16");
                        server.runCommand("item replace entity @a hotbar.1 with minecraft:diamond_sword");
                        server.runCommand("item replace entity @a hotbar.2 with minecraft:bow");
                        server.runCommand("item replace entity @a hotbar.3 with minecraft:golden_apple 8");
                        ctx.waitTicks(5);
                        hold(ctx, 20, W, USE);
                        ctx.waitTicks(3);
                        ctx.getInput().pressKey(hotbar(1));
                        ctx.waitTicks(2);
                        hold(ctx, 15, S, USE);
                        ctx.getInput().pressKey(hotbar(2));
                        ctx.waitTicks(2);
                        ctx.getInput().holdKey(USE);
                        ctx.getInput().holdKey(W);
                        ctx.waitTicks(10);
                        ctx.getInput().pressKey(hotbar(0));
                        ctx.waitTicks(30);
                        ctx.getInput().releaseKey(USE);
                        ctx.getInput().releaseKey(W);
                        ctx.waitTicks(5);
                        server.runCommand("item replace entity @a weapon.offhand with minecraft:shield");
                        ctx.getInput().pressKey(hotbar(1));
                        ctx.waitTicks(3);
                        ctx.getInput().holdKey(USE);
                        ctx.getInput().holdKey(S);
                        ctx.waitTicks(12);
                        ctx.getInput().pressKey(SWAP);
                        ctx.waitTicks(12);
                        ctx.getInput().releaseKey(USE);
                        ctx.getInput().releaseKey(S);
                        ctx.waitTicks(3);
                        ctx.getInput().pressKey(hotbar(3));
                        ctx.waitTicks(2);
                        ctx.getInput().holdKey(USE);
                        ctx.getInput().holdKey(W);
                        ctx.waitTicks(10);
                        ctx.getInput().pressKey(SWAP);
                        ctx.waitTicks(20);
                        ctx.getInput().releaseKey(USE);
                        ctx.getInput().releaseKey(W);
                        ctx.waitTicks(10);
                    }),
            new Scenario("ranged",
                    "Drawing a bow while walking and shooting, charging a crossbow and firing it, readying a trident and throwing it, a spyglass, a goat horn and its cooldown.",
                    List.of(), start(0.5, -60, 0.5, -90, -10), (ctx, server) -> {
                        server.runCommand("item replace entity @a hotbar.0 with minecraft:bow");
                        server.runCommand("item replace entity @a hotbar.1 with minecraft:crossbow");
                        server.runCommand("item replace entity @a hotbar.2 with minecraft:trident");
                        server.runCommand("item replace entity @a hotbar.3 with minecraft:spyglass");
                        server.runCommand("item replace entity @a hotbar.4 with minecraft:goat_horn");
                        server.runCommand("item replace entity @a inventory.0 with minecraft:arrow 16");
                        ctx.waitTicks(5);
                        hold(ctx, 25, W, USE);
                        ctx.waitTicks(5);
                        ctx.getInput().pressKey(hotbar(1));
                        ctx.waitTicks(2);
                        hold(ctx, 32, S, USE);
                        ctx.waitTicks(5);
                        ctx.getInput().pressKey(USE);
                        ctx.waitTicks(8);
                        ctx.getInput().pressKey(hotbar(2));
                        ctx.waitTicks(2);
                        hold(ctx, 15, W, USE);
                        ctx.waitTicks(8);
                        ctx.getInput().pressKey(hotbar(3));
                        ctx.waitTicks(2);
                        hold(ctx, 15, S, USE);
                        ctx.getInput().pressKey(hotbar(4));
                        ctx.waitTicks(2);
                        hold(ctx, 12, W, USE);
                        ctx.waitTicks(3);
                        hold(ctx, 10, S, USE);
                        ctx.waitTicks(5);
                        server.runCommand("kill @e[type=!minecraft:player]");
                        ctx.waitTicks(5);
                    }),
            new Scenario("throwing",
                    "Experience bottles thrown with two clicks a tick, then switching to a golden apple while still clicking, then holding to eat it; snowballs thrown by holding use; ender pearls straight up, held back by their cooldown, landing back on the player.",
                    List.of(), start(0.5, -60, 0.5, -90, -30), (ctx, server) -> {
                        server.runCommand("item replace entity @a hotbar.0 with minecraft:experience_bottle 64");
                        server.runCommand("item replace entity @a hotbar.1 with minecraft:golden_apple 16");
                        server.runCommand("item replace entity @a hotbar.2 with minecraft:snowball 16");
                        server.runCommand("item replace entity @a hotbar.3 with minecraft:ender_pearl 4");
                        ctx.waitTicks(5);
                        ctx.getInput().holdKey(W);
                        for (int i = 0; i < 20; i++) {
                            ctx.getInput().pressKey(USE);
                            ctx.getInput().pressKey(USE);
                            ctx.waitTick();
                        }
                        ctx.getInput().pressKey(hotbar(1));
                        for (int i = 0; i < 30; i++) {
                            ctx.getInput().pressKey(USE);
                            ctx.getInput().pressKey(USE);
                            ctx.waitTick();
                        }
                        hold(ctx, 40, USE);
                        ctx.getInput().releaseKey(W);
                        ctx.waitTicks(5);
                        ctx.getInput().pressKey(hotbar(2));
                        ctx.waitTicks(2);
                        hold(ctx, 20, S, USE);
                        ctx.getInput().pressKey(hotbar(3));
                        ctx.getInput().lookAt(-90, -90);
                        ctx.waitTicks(2);
                        hold(ctx, 30, USE);
                        ctx.waitTicks(40);
                        server.runCommand("kill @e[type=!minecraft:player]");
                        ctx.waitTicks(5);
                    }));
}
