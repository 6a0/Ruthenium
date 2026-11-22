package dev.ruthenium.trace;

import java.util.List;
import java.util.function.Function;
import net.fabricmc.fabric.api.client.gametest.v1.context.ClientGameTestContext;
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

    private static double[] start(double x, double y, double z, double yaw, double pitch) {
        return new double[] {x, y, z, yaw, pitch};
    }

    public static final List<Scenario> ALL = List.of(
            new Scenario("walk_directions",
                    "Walk 15 ticks in each of the eight key directions on stone, 5 idle ticks between.",
                    List.of(), start(0.5, -60, 0.5, 0, 0), ctx -> {
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
                    List.of(), start(0.5, -60, 0.5, -90, 0), ctx -> {
                        hold(ctx, 40, W, SPRINT, JUMP);
                        ctx.waitTicks(20);
                    }),
            new Scenario("sneak_edge",
                    "On a 5x5 platform 3 blocks high: sneak forward into the edge, sneak sideways along it, then walk off without sneaking.",
                    List.of("fill 0 -60 -2 4 -58 2 minecraft:stone"), start(2.5, -57, 0.5, -90, 0), ctx -> {
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
                    start(0.5, -60, 0.5, -90, 0), ctx -> {
                        hold(ctx, 20, W, SPRINT);
                        ctx.waitTicks(60);
                    }),
            new Scenario("slime_bounce",
                    "Walk off a 10-block pillar onto slime, bounce until settled, then jump on the slime.",
                    List.of("fill 1 -61 -2 6 -61 2 minecraft:slime_block", "fill 0 -60 0 0 -51 0 minecraft:stone"),
                    start(0.5, -50, 0.5, -90, 0), ctx -> {
                        hold(ctx, 6, W);
                        ctx.waitTicks(80);
                        hold(ctx, 20, JUMP);
                        ctx.waitTicks(20);
                    }),
            new Scenario("ladder",
                    "Climb a ladder by walking into it, hold on by sneaking, then let go and slide down.",
                    List.of("fill 2 -60 -1 2 -52 1 minecraft:stone", "fill 1 -60 0 1 -53 0 minecraft:ladder[facing=west]"),
                    start(0.5, -60, 0.5, -90, 0), ctx -> {
                        hold(ctx, 40, W);
                        hold(ctx, 20, W, SNEAK);
                        hold(ctx, 20, SNEAK);
                        ctx.waitTicks(40);
                    }),
            new Scenario("swim",
                    "Walk into a 2-deep pool, sprint-swim, then swim up.",
                    List.of("fill 0 -62 -2 12 -61 2 minecraft:water"), start(-1.5, -60, 0.5, -90, 0), ctx -> {
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
                    start(8.5, -60, 0.5, 90, 0), ctx -> {
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
                    start(3.5, -52, 0.5, -90, 0), ctx -> {
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
                    start(-1.5, -60, 0.5, -90, 0), ctx -> {
                        hold(ctx, 30, W);
                        hold(ctx, 50, W, JUMP);
                        ctx.waitTicks(20);
                    }),
            new Scenario("step_up",
                    "Walk over a slab, a carpet, a 3-layer snow block and stairs (stepping up), then jump onto a full block.",
                    List.of("setblock 2 -60 0 minecraft:smooth_stone_slab", "setblock 4 -60 0 minecraft:white_carpet",
                            "setblock 6 -60 0 minecraft:snow[layers=3]", "setblock 8 -60 0 minecraft:oak_stairs[facing=east]",
                            "setblock 10 -60 0 minecraft:stone"),
                    start(0.5, -60, 0.5, -90, 0), ctx -> {
                        hold(ctx, 50, W);
                        hold(ctx, 15, W, JUMP);
                        ctx.waitTicks(10);
                    }),
            new Scenario("head_hitter",
                    "Sprint-jump under a ceiling 2 blocks above the floor.",
                    List.of("fill 0 -58 -1 20 -58 1 minecraft:stone"), start(0.5, -60, 0.5, -90, 0), ctx -> {
                        hold(ctx, 40, W, SPRINT, JUMP);
                        ctx.waitTicks(10);
                    }),
            new Scenario("mouse_turn",
                    "Walk forward while the mouse turns the camera every tick (turns are applied per frame).",
                    List.of(), start(0.5, -60, 0.5, 0, 0), ctx -> {
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
                    List.of("summon minecraft:armor_stand 2.5 -60 0.5",
                            "item replace entity @a hotbar.0 with minecraft:diamond_sword",
                            "item replace entity @a hotbar.1 with minecraft:shield"),
                    start(0.5, -60, 0.5, -90, 0), ctx -> {
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
                    }));
}
