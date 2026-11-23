package dev.ruthenium.trace;

import java.util.List;
import java.util.function.Function;
import net.fabricmc.fabric.api.client.gametest.v1.context.ClientGameTestContext;
import net.fabricmc.fabric.api.client.gametest.v1.context.TestServerContext;
import net.minecraft.client.KeyMapping;
import net.minecraft.client.Options;
import net.minecraft.client.gui.screens.DeathScreen;

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
            "gamerule immediate_respawn false",
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

    /** Teleports the player and waits for the teleport to land. */
    private static void tp(ClientGameTestContext ctx, TestServerContext server, double x, double y, double z,
            double yaw, double pitch) {
        server.runCommand(String.format(java.util.Locale.ROOT, "tp @p %.4f %.4f %.4f %.4f %.4f", x, y, z, yaw, pitch));
        ctx.waitTicks(4);
    }

    /**
     * Puts a redstone block at (x, y, z) to power the piston next to it ({@code on}), or the
     * block the layer had there (stone in the floor at y -61, dirt below, stone above), then
     * waits {@code ticks}.
     */
    private static void power(ClientGameTestContext ctx, TestServerContext server, int x, int y, int z, boolean on,
            int ticks) {
        String off = y == -62 ? "minecraft:dirt" : "minecraft:stone";
        server.runCommand("setblock " + x + " " + y + " " + z + " " + (on ? "minecraft:redstone_block" : off));
        ctx.waitTicks(ticks);
    }

    /** Waits until the client's player has passed x = {@code x} going east (at most 100 ticks). */
    private static void waitUntilEastOf(ClientGameTestContext ctx, double x) {
        waitUntilEastOf(ctx, x, 100);
    }

    /** Waits until the client's player has passed x = {@code x} going east (at most {@code max} ticks). */
    private static void waitUntilEastOf(ClientGameTestContext ctx, double x, int max) {
        for (int i = 0; i < max && ctx.computeOnClient(mc -> mc.player.getX()) < x; i++) {
            ctx.waitTick();
        }
    }

    /** Whether the client's player is a new, living one that has loaded into the level with no screen open. */
    private static boolean backInTheWorld(net.minecraft.client.Minecraft mc) {
        return mc.player != null && mc.player.isAlive() && !mc.player.isDeadOrDying()
                && mc.getConnection() != null && mc.getConnection().hasClientLoaded() && mc.gui.screen() == null;
    }

    /**
     * Waits for the death screen, clicks Respawn once its buttons work (20 ticks after it
     * opened), and waits until the new player is back in the world.
     */
    private static void respawn(ClientGameTestContext ctx) {
        ctx.waitForScreen(DeathScreen.class);
        ctx.waitTicks(25);
        ctx.clickScreenButton("deathScreen.respawn");
        ctx.waitFor(Scenarios::backInTheWorld);
        ctx.waitTicks(5);
    }

    /** Waits until the client's player is on the ground or in water (at most {@code max} ticks). */
    private static void waitUntilDown(ClientGameTestContext ctx, int max) {
        for (int i = 0; i < max && !ctx.computeOnClient(mc -> mc.player.onGround() || mc.player.isInWater()); i++) {
            ctx.waitTick();
        }
    }

    /** Resistance V: no fall or wall damage. */
    private static final String UNHURT = "effect give @a minecraft:resistance infinite 4 true";

    private static final String ELYTRA = "item replace entity @a armor.chest with minecraft:elytra";

    /** Waits until the client's player has passed x = {@code x} going west (at most 100 ticks). */
    private static void waitUntilWestOf(ClientGameTestContext ctx, double x) {
        for (int i = 0; i < 100 && ctx.computeOnClient(mc -> mc.player.getX()) > x; i++) {
            ctx.waitTick();
        }
    }

    /**
     * A block the player meets in {@link #BLOCK_SHAPES_A} and {@link #BLOCK_SHAPES_B}: the
     * state at 4 -60 0 and, for two-block things, the state at 5 -60 0.
     */
    private record Shaped(String state, String east) {
    }

    /**
     * Runs into the block from the west, sprint-jumps onto or over it, then drops onto its top
     * and sneaks to its east, south, west and north edges.
     */
    private static void blockShapes(ClientGameTestContext ctx, TestServerContext server, List<Shaped> blocks) {
        for (var b : blocks) {
            if (b.state().startsWith("minecraft:cactus")) {
                server.runCommand("setblock 4 -61 0 minecraft:sand strict");
            }
            server.runCommand("setblock 4 -60 0 " + b.state() + " strict");
            if (b.east() != null) {
                server.runCommand("setblock 5 -60 0 " + b.east() + " strict");
            }
            tp(ctx, server, 0.5, -60, 0.5, -90, 0);
            hold(ctx, 16, W, SPRINT);
            ctx.waitTicks(3);
            hold(ctx, 10, W, SPRINT, JUMP);
            ctx.waitTicks(10);
            tp(ctx, server, 4.5, -58, 0.5, -90, 0);
            ctx.waitTicks(10);
            hold(ctx, 15, W, SNEAK);
            hold(ctx, 12, D, SNEAK);
            hold(ctx, 25, S, SNEAK);
            hold(ctx, 25, A, SNEAK);
            ctx.waitTicks(3);
            server.runCommand("setblock 4 -60 0 minecraft:air strict");
            server.runCommand("setblock 5 -60 0 minecraft:air strict");
            server.runCommand("setblock 4 -61 0 minecraft:stone strict");
            ctx.waitTicks(2);
        }
    }

    private static final List<Shaped> BLOCK_SHAPES_A = List.of(
            new Shaped("minecraft:cactus", null),
            new Shaped("minecraft:chest[facing=west]", null),
            new Shaped("minecraft:ender_chest[facing=west]", null),
            new Shaped("minecraft:brewing_stand", null),
            new Shaped("minecraft:oak_fence", null),
            new Shaped("minecraft:iron_bars", null),
            new Shaped("minecraft:glass_pane", null),
            new Shaped("minecraft:end_portal_frame[facing=west,eye=false]", null),
            new Shaped("minecraft:end_portal_frame[facing=west,eye=true]", null),
            new Shaped("minecraft:player_head", null),
            new Shaped("minecraft:anvil[facing=west]", null),
            new Shaped("minecraft:white_carpet", null));

    private static final List<Shaped> BLOCK_SHAPES_B = List.of(
            new Shaped("minecraft:oak_stairs[facing=east]", null),
            new Shaped("minecraft:smooth_stone_slab", null),
            new Shaped("minecraft:enchanting_table", null),
            new Shaped("minecraft:hopper", null),
            new Shaped("minecraft:oak_fence_gate[facing=east]", null),
            new Shaped("minecraft:piston[facing=up]", null),
            new Shaped("minecraft:piston[facing=east,extended=true]", "minecraft:piston_head[facing=east]"),
            new Shaped("minecraft:red_bed[facing=east,part=foot]", "minecraft:red_bed[facing=east,part=head]"),
            new Shaped("minecraft:daylight_detector", null),
            new Shaped("minecraft:cobblestone_wall", null),
            new Shaped("minecraft:oak_trapdoor", null),
            new Shaped("minecraft:campfire[lit=false]", null));

    /** A lane along x from -6 up to a stone wall at x = {@code wall} (3 wide, centred on z). */
    private static List<String> lane(int z, String floor, boolean trapdoors, boolean ceiling, int wall) {
        var c = new java.util.ArrayList<String>();
        c.add(String.format("fill -6 -61 %d %d -61 %d %s", z - 1, wall - 1, z + 1, floor));
        if (trapdoors) {
            c.add(String.format("fill -6 -60 %d %d -60 %d minecraft:oak_trapdoor", z - 1, wall - 1, z + 1));
        }
        if (ceiling) {
            c.add(String.format("fill -6 -58 %d %d -58 %d minecraft:stone", z - 1, wall - 1, z + 1));
        }
        c.add(String.format("fill %d -60 %d %d -58 %d minecraft:stone", wall, z - 1, wall, z + 1));
        return c;
    }

    /** Lists joined. */
    @SafeVarargs
    private static List<String> all(List<String>... parts) {
        var c = new java.util.ArrayList<String>();
        for (var p : parts) {
            c.addAll(p);
        }
        return c;
    }

    /** A walled channel of settled flowing water over {@code floor}: the source at x = 0, levels 1 to 7 downstream. */
    private static List<String> channel(int z, String floor) {
        var c = new java.util.ArrayList<String>();
        c.add(String.format("fill -1 -60 %d 9 -59 %d minecraft:stone", z - 2, z + 2));
        c.add(String.format("fill 0 -60 %d 8 -59 %d minecraft:air", z - 1, z + 1));
        c.add(String.format("fill 0 -61 %d 8 -61 %d %s", z - 1, z + 1, floor));
        c.add(String.format("fill 0 -60 %d 0 -60 %d minecraft:water", z - 1, z + 1));
        for (int level = 1; level <= 7; level++) {
            c.add(String.format("fill %d -60 %d %d -60 %d minecraft:water[level=%d]", level, z - 1, level, z + 1, level));
        }
        return c;
    }

    /** The staircase's rows: x = i is 3 wide with its top at y = -51 - i. */
    private static List<String> staircase() {
        var c = new java.util.ArrayList<String>();
        for (int i = 0; i <= 8; i++) {
            c.add(String.format("fill %d -60 -1 %d %d 1 minecraft:stone", i, i, -52 - i));
        }
        return c;
    }

    /** Walks (or sprints) down the staircase, sneaking for 0 to 3 ticks as the player passes each edge. */
    private static void downTheStairs(ClientGameTestContext ctx, boolean sprint, int shift) {
        ctx.getInput().holdKey(W);
        if (sprint) {
            ctx.getInput().holdKey(SPRINT);
        }
        for (int i = 0; i <= 8; i++) {
            waitUntilEastOf(ctx, i + 1.0);
            int sneak = (i + shift) % 4;
            if (sneak > 0) {
                ctx.getInput().holdKey(SNEAK);
                ctx.waitTicks(sneak);
                ctx.getInput().releaseKey(SNEAK);
            }
        }
        ctx.waitTicks(10);
        ctx.getInput().releaseKey(W);
        ctx.getInput().releaseKey(SPRINT);
        ctx.waitTicks(10);
    }

    private static double[] start(double x, double y, double z, double yaw, double pitch) {
        return new double[] {x, y, z, yaw, pitch};
    }


    /** A tame, saddled, still horse with fixed attributes (no random speed or jump). */
    private static final String HORSE = "{Tame:1b,NoAI:1b,PersistenceRequired:1b,Silent:1b,Variant:0,Rotation:[-90f,0f],"
            + "equipment:{saddle:{id:\"minecraft:saddle\",count:1}},attributes:[{id:\"minecraft:movement_speed\",base:0.3d},"
            + "{id:\"minecraft:jump_strength\",base:0.8d}]}";

    /** The same horse without its saddle. */
    private static final String BARE_HORSE = "{Tame:1b,NoAI:1b,PersistenceRequired:1b,Silent:1b,Variant:0,Rotation:[-90f,0f],"
            + "attributes:[{id:\"minecraft:movement_speed\",base:0.3d},{id:\"minecraft:jump_strength\",base:0.8d}]}";

    /** A tame, saddled donkey carrying a chest. */
    private static final String DONKEY = "{Tame:1b,ChestedHorse:1b,NoAI:1b,PersistenceRequired:1b,Silent:1b,Rotation:[-90f,0f],"
            + "equipment:{saddle:{id:\"minecraft:saddle\",count:1}},attributes:[{id:\"minecraft:movement_speed\",base:0.175d},"
            + "{id:\"minecraft:jump_strength\",base:0.5d}]}";

    /** A grown happy ghast in a harness. */
    private static final String GHAST = "{NoAI:1b,PersistenceRequired:1b,Silent:1b,Rotation:[-90f,0f],"
            + "equipment:{body:{id:\"minecraft:white_harness\",count:1}}}";

    /** The same ghast with its AI on (it drifts about). */
    private static final String GHAST_AI = "{PersistenceRequired:1b,Silent:1b,Rotation:[-90f,0f],"
            + "equipment:{body:{id:\"minecraft:white_harness\",count:1}}}";

    /** A tame, saddled, still camel, sitting (it sat down at game tick 1). */
    private static final String CAMEL = "{Tame:1b,NoAI:1b,PersistenceRequired:1b,Silent:1b,LastPoseTick:-1L,Rotation:[-90f,0f],"
            + "equipment:{saddle:{id:\"minecraft:saddle\",count:1}}}";

    /** A saddled, still mob (a pig or a nautilus). */
    private static final String SADDLED = "{NoAI:1b,PersistenceRequired:1b,Silent:1b,Rotation:[-90f,0f],"
            + "equipment:{saddle:{id:\"minecraft:saddle\",count:1}}}";

    /** A saddled strider with its AI on (it gets cold out of the warm only then). */
    private static final String STRIDER = "{PersistenceRequired:1b,Silent:1b,Rotation:[-90f,0f],"
            + "equipment:{saddle:{id:\"minecraft:saddle\",count:1}}}";

    /** The player as an owner reference (`Owner`: its UUID as an int array), to tame a mob. */
    private static String owner(ClientGameTestContext ctx) {
        java.util.UUID u = ctx.computeOnClient(mc -> mc.player.getUUID());
        long m = u.getMostSignificantBits();
        long l = u.getLeastSignificantBits();
        return String.format(java.util.Locale.ROOT, "Owner:[I;%d,%d,%d,%d]", (int) (m >> 32), (int) m, (int) (l >> 32),
                (int) l);
    }

    /** Sits a camel down now (its last pose change, negated, is this game tick). */
    private static final String SIT_CAMEL = "execute as @e[type=minecraft:camel] store result entity @s LastPoseTick long -1 "
            + "run time query gametime";

    /** Mounts the player on the nearest entity of the type with /ride and waits for it to land. */
    private static void ride(ClientGameTestContext ctx, TestServerContext server, String type) {
        server.runCommand("ride @p mount @e[type=" + type + ",limit=1,sort=nearest]");
        ctx.waitTicks(5);
    }

    /** Kills the scenario's vehicles (which drops the player) and waits until they are gone (the dying push for 20 ticks). */
    private static void gone(ClientGameTestContext ctx, TestServerContext server, String type) {
        server.runCommand("kill @e[type=" + type + "]");
        ctx.waitTicks(30);
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
            new Scenario("shallow_water",
                    "Two walled channels of settled flowing water (the source at x = 0, levels 1 to 7 downstream, so about half a block deep midway), one over slime and one over ice: jump off the end wall onto the slime, bounce, walk and jump against the current; then step off onto the ice, walk, sprint, slide and jump.",
                    all(channel(-4, "minecraft:slime_block"), channel(4, "minecraft:ice")),
                    start(9.5, -58, -3.5, 90, 0), (ctx, server) -> {
                        hold(ctx, 6, W, SPRINT, JUMP);
                        ctx.waitTicks(30);
                        hold(ctx, 30, W);
                        hold(ctx, 20, W, JUMP);
                        ctx.waitTicks(10);
                        tp(ctx, server, 9.5, -58, 4.5, 90, 0);
                        hold(ctx, 10, W);
                        ctx.waitTicks(10);
                        hold(ctx, 15, W);
                        hold(ctx, 15, W, SPRINT);
                        ctx.waitTicks(15);
                        hold(ctx, 10, JUMP);
                        ctx.waitTicks(15);
                    }),
            new Scenario("swim_under",
                    "A 2-deep pool with a 1-high stretch under waterlogged bottom slabs (x 4 to 7) and lily pads on the surface (x 10 to 12): sink, sprint-swim along the bottom under the slabs, let go of forward there (the swim sprint carries on), swim on under the lily pads and stop, swim up into them (the pose drops to crouching), then forward.",
                    List.of("fill 0 -62 -1 14 -61 1 minecraft:water",
                            "fill 4 -61 -1 7 -61 1 minecraft:smooth_stone_slab[waterlogged=true]",
                            "fill 10 -60 -1 12 -60 1 minecraft:lily_pad"),
                    start(-1.5, -60, 0.5, -90, 0), (ctx, server) -> {
                        hold(ctx, 10, W);
                        ctx.waitTicks(15);
                        ctx.getInput().lookAt(-90, 25);
                        ctx.getInput().holdKey(W);
                        ctx.getInput().holdKey(SPRINT);
                        waitUntilEastOf(ctx, 5.0);
                        ctx.getInput().releaseKey(W);
                        ctx.getInput().releaseKey(SPRINT);
                        // Letting go of forward ends the sprint; under the slabs the pose stays swimming.
                        ctx.waitTicks(6);
                        ctx.getInput().holdKey(W);
                        waitUntilEastOf(ctx, 8.5);
                        ctx.getInput().holdKey(SPRINT);
                        waitUntilEastOf(ctx, 11.0);
                        ctx.getInput().releaseKey(W);
                        ctx.getInput().releaseKey(SPRINT);
                        ctx.waitTicks(15);
                        ctx.getInput().lookAt(-90, 0);
                        hold(ctx, 25, JUMP);
                        hold(ctx, 30, W, JUMP);
                        ctx.waitTicks(15);
                    }),
            new Scenario("crawl",
                    "A block appears at head height, forcing the crawling pose; crawl through a 1-high tunnel (x 2 to 8), jumping and sneaking inside it, then out the far end and stand up.",
                    List.of("fill 2 -60 -1 8 -59 1 minecraft:stone", "fill 2 -60 0 8 -60 0 minecraft:air"),
                    start(1.5, -60, 0.5, -90, 0), (ctx, server) -> {
                        ctx.waitTicks(3);
                        server.runCommand("setblock 1 -59 0 minecraft:stone");
                        ctx.waitTicks(8);
                        hold(ctx, 15, W);
                        hold(ctx, 10, W, JUMP);
                        hold(ctx, 10, W, SNEAK);
                        hold(ctx, 15, W, SPRINT);
                        hold(ctx, 95, W);
                        ctx.waitTicks(15);
                    }),
            new Scenario("fence_slime_water",
                    "A row of alternating slime blocks and waterlogged fences standing in still water 2 deep: sprint-jump along it from the wall, then turn and walk back.",
                    all(List.of("fill -1 -60 -3 14 -59 3 minecraft:stone", "fill 0 -60 -2 13 -59 2 minecraft:water"),
                            java.util.stream.IntStream.rangeClosed(0, 13).mapToObj(x -> x % 2 == 0
                                    ? "setblock " + x + " -60 0 minecraft:slime_block"
                                    : "setblock " + x + " -60 0 minecraft:oak_fence[east=true,west=true,waterlogged=true]").toList()),
                    start(-0.5, -58, 0.5, -90, 0), (ctx, server) -> {
                        hold(ctx, 50, W, SPRINT, JUMP);
                        ctx.waitTicks(10);
                        ctx.getInput().lookAt(90, 0);
                        hold(ctx, 60, W);
                        ctx.waitTicks(10);
                    }),
            new Scenario("ice_lanes",
                    "Four ice lanes, each ending at a wall: sprint on bare ice; on bottom trapdoors laid on ice; sprint-jump under a ceiling 2 blocks above the ice; sprint-jump on trapdoors on ice under the same ceiling. Let go and slide after each.",
                    all(lane(-9, "minecraft:ice", false, false, 1), lane(-3, "minecraft:ice", true, false, 1),
                            lane(3, "minecraft:ice", false, true, 6), lane(9, "minecraft:ice", true, true, 6)),
                    start(-5.5, -60, -8.5, -90, 0), (ctx, server) -> {
                        hold(ctx, 30, W, SPRINT);
                        ctx.waitTicks(30);
                        tp(ctx, server, -5.5, -59.8125, -2.5, -90, 0);
                        hold(ctx, 30, W, SPRINT);
                        ctx.waitTicks(30);
                        tp(ctx, server, -5.5, -60, 3.5, -90, 0);
                        hold(ctx, 30, W, SPRINT, JUMP);
                        ctx.waitTicks(30);
                        tp(ctx, server, -5.5, -59.8125, 9.5, -90, 0);
                        hold(ctx, 30, W, SPRINT, JUMP);
                        ctx.waitTicks(30);
                    }),
            new Scenario("slime_lanes",
                    "Sprint-jump on slime under a ceiling 2 blocks above it, then on bottom trapdoors laid on slime under the same ceiling (each lane ends at a wall); then sneak-walk and walk on the slime.",
                    all(lane(-3, "minecraft:slime_block", false, true, 6), lane(3, "minecraft:slime_block", true, true, -2)),
                    start(-5.5, -60, -2.5, -90, 0), (ctx, server) -> {
                        hold(ctx, 40, W, SPRINT, JUMP);
                        ctx.waitTicks(20);
                        tp(ctx, server, -5.5, -59.8125, 3.5, -90, 0);
                        hold(ctx, 40, W, SPRINT, JUMP);
                        ctx.waitTicks(20);
                        tp(ctx, server, -5.5, -60, -2.5, -90, 0);
                        hold(ctx, 20, W, SNEAK);
                        hold(ctx, 20, W);
                        ctx.waitTicks(5);
                    }),
            new Scenario("soul_sand",
                    "Sprint from stone across five blocks of soul sand and off again, walk back across, then walk and jump onto it.",
                    List.of("fill 4 -61 -1 8 -61 1 minecraft:soul_sand"),
                    start(0.5, -60, 0.5, -90, 0), (ctx, server) -> {
                        hold(ctx, 30, W, SPRINT);
                        ctx.waitTicks(10);
                        ctx.getInput().lookAt(90, 0);
                        hold(ctx, 50, W);
                        ctx.waitTicks(5);
                        ctx.getInput().lookAt(-90, 0);
                        hold(ctx, 8, W);
                        hold(ctx, 20, W, JUMP);
                        ctx.waitTicks(10);
                    }),
            new Scenario("climbing",
                    "A ladder on a 5-high pillar: sprint along the wall into the ladder's edge; sprint into its face and climb to the top; sneak back to the top edge, then walk off. A vine on another pillar: walk into it, climb, climb holding only jump, slide, hold on sneaking, slide down.",
                    List.of("fill 4 -60 -1 4 -56 1 minecraft:stone", "fill 3 -60 0 3 -56 0 minecraft:ladder[facing=west]",
                            "fill 4 -60 5 4 -55 7 minecraft:stone", "fill 3 -60 6 3 -56 6 minecraft:vine[east=true]"),
                    start(3.65, -60, -3.5, 0, 0), (ctx, server) -> {
                        hold(ctx, 15, W, SPRINT);
                        ctx.waitTicks(5);
                        tp(ctx, server, 0.5, -60, 0.5, -90, 0);
                        hold(ctx, 20, W, SPRINT);
                        hold(ctx, 50, W);
                        ctx.waitTicks(5);
                        ctx.getInput().lookAt(90, 0);
                        hold(ctx, 20, W, SNEAK);
                        ctx.waitTicks(5);
                        hold(ctx, 10, W);
                        ctx.waitTicks(30);
                        tp(ctx, server, 0.5, -60, 6.5, -90, 0);
                        hold(ctx, 15, W);
                        hold(ctx, 20, W);
                        hold(ctx, 20, JUMP);
                        ctx.waitTicks(8);
                        hold(ctx, 15, SNEAK);
                        ctx.waitTicks(40);
                    }),
            new Scenario("stair_sneak",
                    "Down a staircase of 3-wide rows, each a block lower than the last: walking, then sprinting, the player sneaks for 0 to 3 ticks as it passes each edge, carrying its speed off the step.",
                    staircase(), start(0.5, -51, 0.5, -90, 0), (ctx, server) -> {
                        downTheStairs(ctx, false, 0);
                        tp(ctx, server, 0.5, -51, 0.5, -90, 0);
                        downTheStairs(ctx, true, 2);
                    }),
            new Scenario("water_cube",
                    "A lone water source 1 block above the ground, set without updates so it doesn't flow: walk into it, jump and swim in it, sprint through it, come back and sink out of it.",
                    List.of("setblock 3 -59 0 minecraft:water strict"),
                    start(0.5, -60, 0.5, -90, 0), (ctx, server) -> {
                        hold(ctx, 10, W);
                        hold(ctx, 30, JUMP);
                        hold(ctx, 15, W, SPRINT);
                        ctx.waitTicks(10);
                        ctx.getInput().lookAt(90, 0);
                        hold(ctx, 12, W);
                        hold(ctx, 15, W, JUMP);
                        ctx.waitTicks(20);
                        server.runCommand("setblock 3 -59 0 minecraft:air strict");
                        ctx.waitTicks(2);
                    }),
            new Scenario("block_shapes_a",
                    "Each block alone at 4 -60 0 (cactus, chest, ender chest, brewing stand, fence, iron bars, glass pane, end portal frames without and with an eye, player head, anvil, carpet): sprint into it from the west, sprint-jump onto or over it, then drop onto its top and sneak to its east, south, west and north edges.",
                    List.of(), start(0.5, -60, 0.5, -90, 0), (ctx, server) -> blockShapes(ctx, server, BLOCK_SHAPES_A)),
            new Scenario("block_shapes_b",
                    "As block_shapes_a with stairs, a slab, an enchanting table, a hopper, a fence gate, a retracted piston, an extended piston with its head, a bed, a daylight detector, a wall, a trapdoor and an unlit campfire.",
                    List.of(), start(0.5, -60, 0.5, -90, 0), (ctx, server) -> blockShapes(ctx, server, BLOCK_SHAPES_B)),
            new Scenario("bounce",
                    "Drop onto a bed from 5 blocks, then jump on it; with a bounciness attribute of 0.5, drop onto stone, onto a honey block (which suppresses bouncing), and onto stone sneaking.",
                    List.of("setblock 4 -60 0 minecraft:red_bed[facing=east,part=foot]",
                            "setblock 5 -60 0 minecraft:red_bed[facing=east,part=head]", "setblock 4 -60 6 minecraft:honey_block"),
                    start(1.5, -60, 0.5, -90, 0), (ctx, server) -> {
                        tp(ctx, server, 4.5, -55, 0.5, -90, 0);
                        ctx.waitTicks(40);
                        hold(ctx, 30, JUMP);
                        ctx.waitTicks(10);
                        server.runCommand("attribute @p minecraft:bounciness base set 0.5");
                        ctx.waitTicks(2);
                        tp(ctx, server, 1.5, -55, 3.5, -90, 0);
                        ctx.waitTicks(40);
                        tp(ctx, server, 4.5, -55, 6.5, -90, 0);
                        ctx.waitTicks(30);
                        tp(ctx, server, 1.5, -55, 3.5, -90, 0);
                        hold(ctx, 30, SNEAK);
                        server.runCommand("attribute @p minecraft:bounciness base reset");
                        ctx.waitTicks(5);
                    }),
            new Scenario("speed_zero",
                    "With a movement speed of 0: walk, jump, sprint-jump. Then walk normally and have the speed set to 0 mid-walk while still holding forward.",
                    List.of(), start(0.5, -60, 0.5, -90, 0), (ctx, server) -> {
                        server.runCommand("attribute @p minecraft:movement_speed base set 0");
                        ctx.waitTicks(3);
                        hold(ctx, 20, W);
                        hold(ctx, 15, W, JUMP);
                        hold(ctx, 20, W, SPRINT, JUMP);
                        ctx.waitTicks(5);
                        server.runCommand("attribute @p minecraft:movement_speed base reset");
                        ctx.waitTicks(3);
                        ctx.getInput().holdKey(W);
                        ctx.waitTicks(10);
                        server.runCommand("attribute @p minecraft:movement_speed base set 0");
                        ctx.waitTicks(20);
                        ctx.getInput().releaseKey(W);
                        server.runCommand("attribute @p minecraft:movement_speed base reset");
                        ctx.waitTicks(10);
                    }),
            new Scenario("bubble_columns",
                    "A walled tank 5 deep (water x 2 to 14), with a soul sand bubble column at x = 2 and a magma one at x = 6, both up to the surface: jump off the west wall into the up column's edge; drop into the middle of the up column from 3 blocks above it and bob over it; drop into the down column and get dragged to the bottom; sprint-swim east along the bottom, swim up, dive, swim up to the surface; sprint-swim west back through both columns (dragged down, then pushed up against the wall); hold jump in the up column.",
                    List.of("effect give @a minecraft:fire_resistance 300 0 true", "effect give @a minecraft:water_breathing 300 0 true",
                            "fill 1 -61 -2 15 -56 2 minecraft:stone", "fill 2 -60 -1 14 -56 1 minecraft:water",
                            "setblock 2 -61 0 minecraft:soul_sand", "fill 2 -60 0 2 -56 0 minecraft:bubble_column[drag=false]",
                            "setblock 6 -61 0 minecraft:magma_block", "fill 6 -60 0 6 -56 0 minecraft:bubble_column[drag=true]"),
                    start(1.5, -55, 0.5, -90, 0), (ctx, server) -> {
                        hold(ctx, 3, W, JUMP);
                        ctx.waitTicks(30);
                        tp(ctx, server, 2.5, -52, 0.5, -90, 0);
                        ctx.waitTicks(40);
                        tp(ctx, server, 6.5, -52, 0.5, -90, 0);
                        ctx.waitTicks(40);
                        ctx.getInput().holdKey(W);
                        ctx.getInput().holdKey(SPRINT);
                        waitUntilEastOf(ctx, 9.0);
                        ctx.getInput().lookAt(-90, -45);
                        ctx.waitTicks(8);
                        ctx.getInput().lookAt(-90, 45);
                        ctx.waitTicks(6);
                        ctx.getInput().lookAt(-90, -45);
                        ctx.waitTicks(20);
                        ctx.getInput().releaseKey(W);
                        ctx.getInput().releaseKey(SPRINT);
                        ctx.waitTicks(5);
                        ctx.getInput().lookAt(90, 0);
                        ctx.getInput().holdKey(W);
                        ctx.getInput().holdKey(SPRINT);
                        waitUntilWestOf(ctx, 2.5);
                        ctx.getInput().releaseKey(W);
                        ctx.getInput().releaseKey(SPRINT);
                        hold(ctx, 30, JUMP);
                        ctx.waitTicks(20);
                    }),
            new Scenario("cobwebs",
                    "Three lanes with a cobweb at x = 4: on the floor over soul sand, sprint into it and on through (sinking into the soul sand); at head height, walk into it and on through, then sprint-jump into it and on through; 2 blocks up, sprint-jump into it once and keep sprinting until out of it. Then with weaving, walk through the first again.",
                    List.of("setblock 4 -61 0 minecraft:soul_sand", "setblock 4 -60 0 minecraft:cobweb",
                            "setblock 4 -59 4 minecraft:cobweb", "setblock 4 -58 8 minecraft:cobweb"),
                    start(0.5, -60, 0.5, -90, 0), (ctx, server) -> {
                        ctx.getInput().holdKey(W);
                        ctx.getInput().holdKey(SPRINT);
                        waitUntilEastOf(ctx, 5.4, 300);
                        ctx.waitTicks(5);
                        ctx.getInput().releaseKey(W);
                        ctx.getInput().releaseKey(SPRINT);
                        ctx.waitTicks(5);
                        tp(ctx, server, 0.5, -60, 4.5, -90, 0);
                        ctx.getInput().holdKey(W);
                        waitUntilEastOf(ctx, 5.4, 150);
                        ctx.waitTicks(5);
                        ctx.getInput().releaseKey(W);
                        tp(ctx, server, 0.5, -60, 4.5, -90, 0);
                        hold(ctx, 5, W, SPRINT);
                        hold(ctx, 2, W, SPRINT, JUMP);
                        ctx.getInput().holdKey(W);
                        ctx.getInput().holdKey(SPRINT);
                        waitUntilEastOf(ctx, 5.4, 300);
                        ctx.waitTicks(5);
                        ctx.getInput().releaseKey(W);
                        ctx.getInput().releaseKey(SPRINT);
                        tp(ctx, server, 0.5, -60, 8.5, -90, 0);
                        hold(ctx, 5, W, SPRINT);
                        hold(ctx, 2, W, SPRINT, JUMP);
                        ctx.getInput().holdKey(W);
                        ctx.getInput().holdKey(SPRINT);
                        waitUntilEastOf(ctx, 5.4, 300);
                        ctx.waitTicks(10);
                        ctx.getInput().releaseKey(W);
                        ctx.getInput().releaseKey(SPRINT);
                        ctx.waitTicks(5);
                        server.runCommand("effect give @a minecraft:weaving 60 0");
                        tp(ctx, server, 0.5, -60, 0.5, -90, 0);
                        hold(ctx, 50, W);
                        server.runCommand("effect clear @a");
                        ctx.waitTicks(5);
                    }),
            new Scenario("berry_bushes",
                    "Sweet berry bushes aged 0 to 3 at x = 3 to 6: walk east through them, sprint-jump back west through them, then walk into the oldest and stand in it.",
                    List.of("fill 2 -61 -1 7 -61 1 minecraft:grass_block", "setblock 3 -60 0 minecraft:sweet_berry_bush[age=0]",
                            "setblock 4 -60 0 minecraft:sweet_berry_bush[age=1]", "setblock 5 -60 0 minecraft:sweet_berry_bush[age=2]",
                            "setblock 6 -60 0 minecraft:sweet_berry_bush[age=3]"),
                    start(0.5, -60, 0.5, -90, 0), (ctx, server) -> {
                        ctx.getInput().holdKey(W);
                        waitUntilEastOf(ctx, 7.4, 150);
                        ctx.getInput().releaseKey(W);
                        ctx.waitTicks(5);
                        ctx.getInput().lookAt(90, 0);
                        ctx.getInput().holdKey(W);
                        ctx.getInput().holdKey(SPRINT);
                        ctx.getInput().holdKey(JUMP);
                        waitUntilWestOf(ctx, 2.6);
                        ctx.getInput().releaseKey(W);
                        ctx.getInput().releaseKey(SPRINT);
                        ctx.getInput().releaseKey(JUMP);
                        ctx.waitTicks(10);
                        tp(ctx, server, 8.5, -60, 0.5, 90, 0);
                        hold(ctx, 15, W);
                        ctx.waitTicks(20);
                    }),
            new Scenario("powder_snow",
                    "A 1-deep pit of powder snow (x 4 to 8): walk in (sinking and slowed) and on against its far side, then hold jump there (no way out without boots); fall into it from 6 blocks up (caught by the falling shape, then sinking); in leather boots walk onto its top and along it, sneak to sink in, then walk against the far side and climb out onto the ground beyond.",
                    List.of("fill 4 -61 -1 8 -61 1 minecraft:powder_snow"),
                    start(0.5, -60, 0.5, -90, 0), (ctx, server) -> {
                        ctx.getInput().holdKey(W);
                        waitUntilEastOf(ctx, 8.6);
                        ctx.waitTicks(5);
                        ctx.getInput().releaseKey(W);
                        hold(ctx, 20, W, JUMP);
                        ctx.waitTicks(10);
                        tp(ctx, server, 6.5, -54, 0.5, -90, 0);
                        ctx.waitTicks(40);
                        server.runCommand("item replace entity @p armor.feet with minecraft:leather_boots");
                        tp(ctx, server, 2.5, -60, 0.5, -90, 0);
                        hold(ctx, 15, W);
                        hold(ctx, 10, SNEAK);
                        ctx.getInput().holdKey(W);
                        waitUntilEastOf(ctx, 9.5);
                        ctx.waitTicks(10);
                        ctx.getInput().releaseKey(W);
                        ctx.waitTicks(10);
                        server.runCommand("clear @p");
                        ctx.waitTicks(3);
                    }),
            new Scenario("honey_slide",
                    "On top of an 8-high honey pillar: jump on it, walk west off its edge and hold back against its side to slide down; again from the top without holding back (falling past it).",
                    List.of("fill 4 -60 0 4 -53 0 minecraft:honey_block"),
                    start(4.5, -52.0625, 0.5, 90, 0), (ctx, server) -> {
                        hold(ctx, 10, JUMP);
                        ctx.waitTicks(10);
                        hold(ctx, 8, W);
                        hold(ctx, 40, S);
                        ctx.waitTicks(10);
                        tp(ctx, server, 4.5, -52.0625, 0.5, 90, 0);
                        hold(ctx, 8, W);
                        ctx.waitTicks(30);
                    }),
            new Scenario("death_screen",
                    "Sprint east holding W and die (/kill) still holding it: the death screen opens and lets go of the keys, the server's motion packet stops the body, and 20 ticks later the client takes the player out of the level (no more ticks or movement packets); Respawn once its buttons work, back at the spawn point (4 -60 4), and walk south. Then jump, die in the air (the body falls to the ground), respawn the same way and walk.",
                    List.of("spawnpoint @a 4 -60 4"),
                    start(0.5, -60, 0.5, -90, 0), (ctx, server) -> {
                        ctx.getInput().holdKey(W);
                        ctx.getInput().holdKey(SPRINT);
                        ctx.waitTicks(10);
                        server.runCommand("kill @p");
                        ctx.waitTicks(30);
                        ctx.getInput().releaseKey(SPRINT);
                        ctx.getInput().releaseKey(W);
                        respawn(ctx);
                        hold(ctx, 10, W);
                        ctx.waitTicks(5);
                        hold(ctx, 4, JUMP);
                        server.runCommand("kill @p");
                        ctx.waitTicks(30);
                        respawn(ctx);
                        hold(ctx, 10, W);
                        ctx.waitTicks(5);
                    }),
            new Scenario("death_immediate",
                    "With immediate_respawn on: walk east holding W and die (/kill): the client asks to respawn while handling the death and again from Gui.tick the next tick (dead, no screen open), and is back at the spawn point (4 -60 4) the tick after, still walking (closing the level loading screen reads the held keys again). Then jump, die in the air, and come back the same way.",
                    List.of("spawnpoint @a 4 -60 4", "gamerule immediate_respawn true"),
                    start(0.5, -60, 0.5, -90, 0), (ctx, server) -> {
                        ctx.getInput().holdKey(W);
                        ctx.waitTicks(10);
                        server.runCommand("kill @p");
                        ctx.waitTicks(2);
                        ctx.waitFor(Scenarios::backInTheWorld);
                        ctx.getInput().releaseKey(W);
                        hold(ctx, 10, W);
                        ctx.waitTicks(5);
                        hold(ctx, 4, JUMP);
                        server.runCommand("kill @p");
                        ctx.waitTicks(2);
                        ctx.waitFor(Scenarios::backInTheWorld);
                        hold(ctx, 10, W);
                        ctx.waitTicks(5);
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
                    }),
            new Scenario("elytra_glide",
                    "Wearing an elytra: jump, then press jump again in the air to glide down to the floor, landing and sliding until the server ends the glide. Then from 20 blocks up, glide east diving (pitch 30), pull up (pitch -20: the descent flattens), glide through a curtain of weeping vines at x = 14 (gliding doesn't climb them), dive again and fly into a stone wall at x = 31 (resistance V: no damage).",
                    List.of(ELYTRA, UNHURT,
                            "fill 14 -50 -3 14 -50 3 minecraft:stone",
                            "fill 14 -56 -3 14 -51 3 minecraft:weeping_vines_plant",
                            "fill 14 -57 -3 14 -57 3 minecraft:weeping_vines[age=25]",
                            "fill 31 -60 -6 31 -52 6 minecraft:stone"),
                    start(-6.5, -60, 0.5, -90, 0), (ctx, server) -> {
                        hold(ctx, 2, JUMP);
                        ctx.waitTicks(3);
                        hold(ctx, 2, JUMP);
                        ctx.waitTicks(30);
                        tp(ctx, server, -6.5, -40, 0.5, -90, 30);
                        hold(ctx, 1, JUMP);
                        ctx.waitTicks(16);
                        ctx.getInput().lookAt(-90, -20);
                        ctx.waitTicks(15);
                        ctx.getInput().lookAt(-90, 30);
                        waitUntilDown(ctx, 100);
                        ctx.waitTicks(15);
                    }),
            new Scenario("elytra_death",
                    "Wearing an elytra, glide east from 20 blocks up and die (/kill) mid-glide: the server ends the glide as the player dies, the body falls, and 20 ticks after dying the client takes it out of the level in mid-air, where it stops; respawn once the death screen's buttons work, back at the spawn point (4 -60 4), and walk.",
                    List.of(ELYTRA, "spawnpoint @a 4 -60 4"),
                    start(-6.5, -60, 0.5, -90, 0), (ctx, server) -> {
                        tp(ctx, server, -6.5, -40, 0.5, -90, 20);
                        hold(ctx, 1, JUMP);
                        ctx.waitTicks(12);
                        server.runCommand("kill @p");
                        ctx.waitTicks(40);
                        respawn(ctx);
                        hold(ctx, 10, W);
                        ctx.waitTicks(5);
                    }),
            new Scenario("elytra_water",
                    "Wearing an elytra, glide from 14 blocks up into a two-deep pool (x 4 to 26): the glide goes on in the water (the server ends it only on the ground), so sneak down to the pool's floor to end it. Then hold W and press jump every few ticks: in the water jump doesn't start a glide, but bobbing out of it at the surface it does, and that glide goes on in the water. Then swim to the pool's east edge and jump out onto the floor still pressing jump.",
                    List.of(ELYTRA, UNHURT, "fill 4 -62 -4 26 -61 4 minecraft:water"),
                    start(-6.5, -60, 0.5, -90, 0), (ctx, server) -> {
                        tp(ctx, server, -6.5, -46, 0.5, -90, 30);
                        hold(ctx, 1, JUMP);
                        waitUntilDown(ctx, 100);
                        ctx.waitTicks(5);
                        ctx.getInput().lookAt(-90, 0);
                        hold(ctx, 25, SNEAK);
                        ctx.waitTicks(5);
                        ctx.getInput().holdKey(W);
                        for (int i = 0; i < 25; i++) {
                            hold(ctx, 2, JUMP);
                            ctx.waitTicks(2);
                        }
                        waitUntilEastOf(ctx, 26.5, 200);
                        for (int i = 0; i < 6; i++) {
                            hold(ctx, 2, JUMP);
                            ctx.waitTicks(2);
                        }
                        ctx.getInput().releaseKey(W);
                        waitUntilDown(ctx, 60);
                        ctx.waitTicks(10);
                    }),
            new Scenario("elytra_fireworks",
                    "Wearing an elytra: glide from 10 blocks up, then use a firework rocket looking up (pitch -70): the client ticks the rocket attached to the player, which boosts it along its look while it glides. Then circle down turning 4 degrees a tick at pitch 15, using a second rocket on the way, until landing.",
                    List.of(ELYTRA, UNHURT, "item replace entity @a hotbar.0 with minecraft:firework_rocket 16"),
                    start(0.5, -60, 0.5, -90, 0), (ctx, server) -> {
                        tp(ctx, server, 0.5, -50, 0.5, -90, -70);
                        hold(ctx, 2, JUMP);
                        ctx.waitTicks(1);
                        use(ctx);
                        ctx.waitTicks(30);
                        float yaw = -90;
                        for (int i = 0; i < 400 && !ctx.computeOnClient(mc -> mc.player.onGround()); i++) {
                            yaw += 4;
                            ctx.getInput().lookAt(yaw, 15);
                            if (i == 40) {
                                ctx.getInput().pressKey(USE);
                            }
                            ctx.waitTick();
                        }
                        ctx.waitTicks(15);
                    }),
            new Scenario("riptide",
                    "A trident with riptide III at the bottom of a two-deep pool (x 0 to 10): held too briefly (nothing); held 12 ticks looking east, launching along the look into the pool's east wall (the wall ends the spin); back at the west side looking up (pitch -60) from the bottom, launched up out of the water (lifted 1.2 first, being on the ground), spinning (the server's flag gives the spin pose) and falling back; then wearing an elytra, launched up again and pressing jump in the air to glide down steeply (pitch 45).",
                    List.of(UNHURT, "fill 0 -62 -4 10 -61 4 minecraft:water"),
                    start(-2.5, -60, 0.5, -90, 0), (ctx, server) -> {
                        server.runCommand("item replace entity @a hotbar.0 with minecraft:trident[enchantments={riptide:3}]");
                        tp(ctx, server, 2.5, -62, 0.5, -90, 0);
                        ctx.waitTicks(10);
                        hold(ctx, 6, USE);
                        ctx.waitTicks(5);
                        hold(ctx, 12, USE);
                        ctx.waitTicks(25);
                        tp(ctx, server, 2.5, -62, 0.5, -90, -60);
                        ctx.waitTicks(10);
                        hold(ctx, 12, USE);
                        ctx.waitTicks(10);
                        waitUntilDown(ctx, 100);
                        ctx.waitTicks(20);
                        server.runCommand(ELYTRA);
                        tp(ctx, server, 2.5, -62, 0.5, -90, -70);
                        ctx.waitTicks(10);
                        hold(ctx, 12, USE);
                        ctx.waitTicks(8);
                        hold(ctx, 2, JUMP);
                        ctx.getInput().lookAt(-90, 45);
                        ctx.waitTicks(5);
                        waitUntilDown(ctx, 120);
                        ctx.waitTicks(15);
                    }),
            new Scenario("wind_charge",
                    "Wearing an elytra: jump looking straight down and throw a wind charge at the floor near the top of the jump: the explosion's knockback (the server's explode packet) launches the player, who presses jump in the air to glide east and lands. Then the same without gliding.",
                    List.of(ELYTRA, UNHURT, "item replace entity @a hotbar.0 with minecraft:wind_charge 16"),
                    start(0.5, -60, 0.5, -90, 90), (ctx, server) -> {
                        hold(ctx, 2, JUMP);
                        ctx.waitTicks(3);
                        ctx.getInput().pressKey(USE);
                        ctx.waitTicks(6);
                        ctx.getInput().lookAt(-90, 10);
                        hold(ctx, 2, JUMP);
                        ctx.waitTicks(2);
                        waitUntilDown(ctx, 120);
                        ctx.waitTicks(15);
                        ctx.getInput().lookAt(-90, 90);
                        ctx.waitTicks(3);
                        hold(ctx, 2, JUMP);
                        ctx.waitTicks(3);
                        ctx.getInput().pressKey(USE);
                        ctx.waitTicks(4);
                        waitUntilDown(ctx, 120);
                        ctx.waitTicks(15);
                        server.runCommand("kill @e[type=!minecraft:player]");
                        ctx.waitTicks(5);
                    }),
            new Scenario("piston_up",
                    "Pistons facing up set into the floor, powered by a redstone block put in the dirt under them: standing on one as its head extends (lifted a block onto the head) and retracts (falling back); on a stone block that a piston pushes up (it stays up); on stone on a sticky piston (pulled back down); standing over a piston's edge (x 3.2, overlapping it by 0.1); walking east across one as it extends under the player.",
                    List.of("setblock 2 -61 0 minecraft:piston[facing=up]",
                            "setblock 5 -61 0 minecraft:piston[facing=up]", "setblock 5 -60 0 minecraft:stone",
                            "setblock 8 -61 0 minecraft:sticky_piston[facing=up]", "setblock 8 -60 0 minecraft:stone",
                            "setblock 2 -61 4 minecraft:piston[facing=up]"),
                    start(2.5, -60, 0.5, -90, 0), (ctx, server) -> {
                        ctx.waitTicks(5);
                        power(ctx, server, 2, -62, 0, true, 20);
                        power(ctx, server, 2, -62, 0, false, 20);
                        tp(ctx, server, 5.5, -59, 0.5, -90, 0);
                        power(ctx, server, 5, -62, 0, true, 20);
                        power(ctx, server, 5, -62, 0, false, 20);
                        tp(ctx, server, 8.5, -59, 0.5, -90, 0);
                        power(ctx, server, 8, -62, 0, true, 20);
                        power(ctx, server, 8, -62, 0, false, 20);
                        tp(ctx, server, 3.2, -60, 0.5, -90, 0);
                        power(ctx, server, 2, -62, 0, true, 20);
                        power(ctx, server, 2, -62, 0, false, 20);
                        tp(ctx, server, 0.5, -60, 4.5, -90, 0);
                        ctx.getInput().holdKey(W);
                        waitUntilEastOf(ctx, 2.2);
                        power(ctx, server, 2, -62, 4, true, 3);
                        ctx.getInput().releaseKey(W);
                        ctx.waitTicks(20);
                        power(ctx, server, 2, -62, 4, false, 20);
                    }),
            new Scenario("piston_side",
                    "Pistons facing east on the floor, powered by a redstone block put in the floor under them: the head pushing the player standing in front of it; the same on ice; walking west into a piston as its head comes out (pushed back while walking against it), then walking on as it retracts; a stone block pushed into the player.",
                    List.of("setblock 0 -60 0 minecraft:piston[facing=east]",
                            "setblock 0 -60 4 minecraft:piston[facing=east]", "fill 1 -61 4 8 -61 4 minecraft:ice",
                            "setblock 0 -60 8 minecraft:piston[facing=east]",
                            "setblock 0 -60 -6 minecraft:piston[facing=east]", "setblock 1 -60 -6 minecraft:stone"),
                    start(1.5, -60, 0.5, 90, 0), (ctx, server) -> {
                        ctx.waitTicks(5);
                        power(ctx, server, 0, -61, 0, true, 20);
                        power(ctx, server, 0, -61, 0, false, 10);
                        tp(ctx, server, 1.5, -60, 4.5, 90, 0);
                        power(ctx, server, 0, -61, 4, true, 20);
                        power(ctx, server, 0, -61, 4, false, 10);
                        tp(ctx, server, 4.5, -60, 8.5, 90, 0);
                        ctx.getInput().holdKey(W);
                        waitUntilWestOf(ctx, 1.6);
                        power(ctx, server, 0, -61, 8, true, 20);
                        power(ctx, server, 0, -61, 8, false, 10);
                        ctx.getInput().releaseKey(W);
                        ctx.waitTicks(5);
                        tp(ctx, server, 2.5, -60, -5.5, 90, 0);
                        power(ctx, server, 0, -61, -6, true, 20);
                        power(ctx, server, 0, -61, -6, false, 10);
                    }),
            new Scenario("piston_crawl",
                    "A 1-high tunnel (x 2 to 10) entered crawling (a block appears at head height). In its ceiling at x 5 an extended piston facing up (powered by a redstone block in the wall), crawled under, retracted and extended again over the player; in its south wall at x 8 a piston facing north, which pushes the crawling player north into a gap in the north wall and retracts; then crawl back into the tunnel and out of its far end.",
                    List.of("fill 2 -60 -1 10 -59 1 minecraft:stone", "fill 2 -60 0 10 -60 0 minecraft:air",
                            "setblock 5 -59 1 minecraft:redstone_block",
                            "setblock 5 -59 0 minecraft:piston[facing=up,extended=true]",
                            "setblock 5 -58 0 minecraft:piston_head[facing=up]",
                            "setblock 8 -60 1 minecraft:piston[facing=north]", "setblock 8 -60 -1 minecraft:air"),
                    start(1.5, -60, 0.5, -90, 0), (ctx, server) -> {
                        ctx.waitTicks(3);
                        server.runCommand("setblock 1 -59 0 minecraft:stone");
                        ctx.waitTicks(8);
                        ctx.getInput().holdKey(W);
                        waitUntilEastOf(ctx, 5.5);
                        ctx.getInput().releaseKey(W);
                        ctx.waitTicks(5);
                        power(ctx, server, 5, -59, 1, false, 10);
                        power(ctx, server, 5, -59, 1, true, 10);
                        ctx.getInput().holdKey(W);
                        waitUntilEastOf(ctx, 8.5);
                        ctx.getInput().releaseKey(W);
                        ctx.waitTicks(5);
                        power(ctx, server, 8, -61, 1, true, 20);
                        power(ctx, server, 8, -61, 1, false, 10);
                        ctx.getInput().lookAt(0, 0);
                        ctx.waitTicks(2);
                        hold(ctx, 12, W);
                        ctx.getInput().lookAt(-90, 0);
                        ctx.waitTicks(2);
                        ctx.getInput().holdKey(W);
                        waitUntilEastOf(ctx, 12.5);
                        ctx.getInput().releaseKey(W);
                        ctx.waitTicks(15);
                    }),
            new Scenario("piston_jump",
                    "Standing on a piston facing up set into the floor: jumping, with its head extending under the player 0, 2, 4, 6, 8 and 10 ticks after the jump; landing on the head, then retracting it.",
                    List.of("setblock 2 -61 0 minecraft:piston[facing=up]"),
                    start(2.5, -60, 0.5, -90, 0), (ctx, server) -> {
                        ctx.waitTicks(5);
                        for (int delay : new int[] {0, 2, 4, 6, 8, 10}) {
                            hold(ctx, 1, JUMP);
                            ctx.waitTicks(delay);
                            power(ctx, server, 2, -62, 0, true, 25);
                            power(ctx, server, 2, -62, 0, false, 20);
                        }
                    }),
            new Scenario("sticky_slime",
                    "Sticky pistons moving slime and honey blocks: standing on slime pushed up (launched up, landing sneaking), then pulled back down; standing east of slime pushed east (launched east) on stone and on ice, each pulled back after; standing on honey pushed east and pulled back (the honey carries the player both ways). Obsidian under the sideways slime and honey keeps them from sticking to the floor.",
                    List.of("setblock 2 -61 0 minecraft:sticky_piston[facing=up]", "setblock 2 -60 0 minecraft:slime_block",
                            "setblock 0 -60 4 minecraft:sticky_piston[facing=east]", "setblock 1 -60 4 minecraft:slime_block",
                            "fill 1 -61 4 2 -61 4 minecraft:obsidian",
                            "setblock 0 -60 8 minecraft:sticky_piston[facing=east]", "setblock 1 -60 8 minecraft:slime_block",
                            "fill 1 -61 8 2 -61 8 minecraft:obsidian",
                            "fill 3 -61 8 28 -61 8 minecraft:ice", "fill 30 -60 7 30 -58 9 minecraft:stone",
                            "setblock 0 -60 -4 minecraft:sticky_piston[facing=east]", "setblock 1 -60 -4 minecraft:honey_block",
                            "fill 1 -61 -4 2 -61 -4 minecraft:obsidian"),
                    start(2.5, -59, 0.5, -90, 0), (ctx, server) -> {
                        ctx.waitTicks(5);
                        power(ctx, server, 2, -62, 0, true, 45);
                        hold(ctx, 30, SNEAK);
                        power(ctx, server, 2, -62, 0, false, 15);
                        tp(ctx, server, 2.5, -60, 4.5, -90, 0);
                        power(ctx, server, 0, -61, 4, true, 30);
                        power(ctx, server, 0, -61, 4, false, 10);
                        tp(ctx, server, 2.5, -60, 8.5, -90, 0);
                        power(ctx, server, 0, -61, 8, true, 40);
                        power(ctx, server, 0, -61, 8, false, 10);
                        tp(ctx, server, 1.5, -59, -3.5, -90, 0);
                        power(ctx, server, 0, -61, -4, true, 20);
                        power(ctx, server, 0, -61, -4, false, 20);
                    }),
            new Scenario("shulker_box",
                    "Standing on a shulker box facing up and opening it (looking down at it): its lid lifts the player; the box is closed (the client closes its screen) and the player drops back onto it. Then opening a shulker box facing east from its east side: its lid pushes the player east; closed again.",
                    List.of("setblock 2 -60 0 minecraft:shulker_box[facing=up]",
                            "setblock 6 -60 4 minecraft:red_shulker_box[facing=east]"),
                    start(2.5, -59, 0.5, -90, 90), (ctx, server) -> {
                        ctx.waitTicks(5);
                        use(ctx);
                        ctx.waitTicks(20);
                        ctx.runOnClient(mc -> mc.player.closeContainer());
                        ctx.waitTicks(20);
                        aim(ctx, server, 7.5, -60, 4.5, 7.0, -59.5, 4.5);
                        use(ctx);
                        ctx.waitTicks(20);
                        ctx.runOnClient(mc -> mc.player.closeContainer());
                        ctx.waitTicks(15);
                    }),
            new Scenario("boat_paddle",
                    "An oak boat in a 2-deep pool, mounted with /ride: paddling forward, then with the camera turned 150 degrees off its heading (held to 105), forward turning left and right, turning on the spot both ways, backward, forward and back together, left and right together, drifting; sneaking out (onto the boat's top), in again and paddling, then quick in and out twice.",
                    List.of("fill 0 -62 -6 30 -61 6 minecraft:water"),
                    start(1.5, -60, -8.5, -90, 0), (ctx, server) -> {
                        server.runCommand("summon minecraft:oak_boat 3.5 -60.4 0.5 {Rotation:[-90f,0f]}");
                        ctx.waitTicks(20);
                        ride(ctx, server, "minecraft:oak_boat");
                        hold(ctx, 40, W);
                        // Looking 150 degrees off the boat's heading: held to 105.
                        ctx.getInput().lookAt(60, 0);
                        hold(ctx, 10, W);
                        ctx.getInput().lookAt(-90, 0);
                        hold(ctx, 20, W, A);
                        hold(ctx, 20, W, D);
                        hold(ctx, 15, A);
                        hold(ctx, 15, D);
                        hold(ctx, 20, S);
                        hold(ctx, 15, W, S);
                        hold(ctx, 15, A, D);
                        ctx.waitTicks(30);
                        hold(ctx, 3, SNEAK);
                        ctx.waitTicks(20);
                        ride(ctx, server, "minecraft:oak_boat");
                        hold(ctx, 20, W);
                        ctx.waitTicks(10);
                        hold(ctx, 1, SNEAK);
                        ctx.waitTicks(5);
                        ride(ctx, server, "minecraft:oak_boat");
                        ctx.waitTicks(2);
                        hold(ctx, 1, SNEAK);
                        ctx.waitTicks(20);
                        gone(ctx, server, "minecraft:oak_boat");
                    }),
            new Scenario("boat_land",
                    "A boat on land, mounted by clicking it with nothing in hand: paddling across stone, ice and blue ice into a wall, backing and turning; teleported (with the player in it) to soul sand and paddled across it; teleported near the edge of a 4-high ledge and driven off onto slime (bouncing); sneaking out.",
                    List.of("fill 2 -61 -1 12 -61 1 minecraft:ice", "fill 13 -61 -1 24 -61 1 minecraft:blue_ice",
                            "fill 26 -60 -2 26 -58 2 minecraft:stone", "fill 2 -61 5 20 -61 7 minecraft:soul_sand",
                            "fill 0 -57 -9 4 -57 -7 minecraft:stone", "fill 5 -61 -10 12 -61 -6 minecraft:slime_block"),
                    start(0.5, -60, 3.5, -90, 0), (ctx, server) -> {
                        server.runCommand("summon minecraft:oak_boat 0.6 -60 0.5 {Rotation:[-90f,0f]}");
                        ctx.waitTicks(10);
                        aim(ctx, server, 0.5, -60, 3.5, 0.6, -59.8, 0.5);
                        use(ctx);
                        ctx.waitTicks(10);
                        hold(ctx, 80, W);
                        ctx.waitTicks(20);
                        hold(ctx, 20, S);
                        hold(ctx, 20, A);
                        hold(ctx, 15, W, D);
                        ctx.waitTicks(10);
                        server.runCommand("tp @e[type=minecraft:oak_boat] 0.6 -60 6.5 -90 0");
                        ctx.waitTicks(10);
                        hold(ctx, 50, W);
                        ctx.waitTicks(10);
                        server.runCommand("tp @e[type=minecraft:oak_boat] 4.0 -56 -8 -90 0");
                        ctx.waitTicks(10);
                        hold(ctx, 45, W);
                        ctx.waitTicks(40);
                        hold(ctx, 3, SNEAK);
                        ctx.waitTicks(10);
                        gone(ctx, server, "minecraft:oak_boat");
                    }),
            new Scenario("boat_bubbles",
                    "A boat paddled over an upward bubble column (soul sand) in a 2-deep pool and left there until the column launches it, then over a downward one (magma) until it drags the boat down and the player is thrown out.",
                    List.of("fill 0 -62 -4 20 -61 4 minecraft:water", "setblock 6 -63 0 minecraft:soul_sand",
                            "fill 6 -62 0 6 -61 0 minecraft:bubble_column[drag=false]",
                            "setblock 14 -63 0 minecraft:magma_block",
                            "fill 14 -62 0 14 -61 0 minecraft:bubble_column[drag=true]"),
                    start(1.5, -60, -6.5, -90, 0), (ctx, server) -> {
                        server.runCommand("summon minecraft:oak_boat 2.5 -60.4 0.5 {Rotation:[-90f,0f]}");
                        ctx.waitTicks(20);
                        ride(ctx, server, "minecraft:oak_boat");
                        hold(ctx, 9, W);
                        ctx.waitTicks(90);
                        hold(ctx, 20, W);
                        ctx.waitTicks(90);
                        ctx.waitTicks(10);
                        gone(ctx, server, "minecraft:oak_boat");
                    }),
            new Scenario("boat_blocks",
                    "A boat on stone: paddled into cobwebs with forward held through them, then backed out; coasting into them with nothing held; paddled onto a pit of powder snow; pushed sideways by a piston.",
                    List.of("fill 6 -60 -1 6 -60 1 minecraft:cobweb", "fill 3 -61 5 10 -61 7 minecraft:powder_snow",
                            "setblock 2 -60 -6 minecraft:piston[facing=east]"),
                    start(0.5, -60, 3.5, -90, 0), (ctx, server) -> {
                        server.runCommand("summon minecraft:oak_boat 0.8 -60 0.5 {Rotation:[-90f,0f]}");
                        ctx.waitTicks(10);
                        ride(ctx, server, "minecraft:oak_boat");
                        hold(ctx, 60, W);
                        hold(ctx, 40, S);
                        ctx.waitTicks(10);
                        server.runCommand("tp @e[type=minecraft:oak_boat] 0.8 -60 0.5 -90 0");
                        ctx.waitTicks(5);
                        hold(ctx, 25, W);
                        ctx.waitTicks(40);
                        server.runCommand("tp @e[type=minecraft:oak_boat] 0.8 -60 6.5 -90 0");
                        ctx.waitTicks(5);
                        hold(ctx, 40, W);
                        ctx.waitTicks(20);
                        hold(ctx, 20, S);
                        server.runCommand("tp @e[type=minecraft:oak_boat] 4.2 -60 -5.5 -90 0");
                        ctx.waitTicks(10);
                        power(ctx, server, 2, -61, -6, true, 20);
                        power(ctx, server, 2, -61, -6, false, 10);
                        hold(ctx, 3, SNEAK);
                        ctx.waitTicks(10);
                        gone(ctx, server, "minecraft:oak_boat");
                    }),
            new Scenario("horse_ride",
                    "A tame saddled horse (speed 0.3, jump strength 0.8): walking up a 1-block step, sprint held (horses can't sprint), jumps charged for 1, 5, 9, 10 and 20 ticks, a charged jump while walking, forward and back together, left and right together, strafing, turning and walking; through cobwebs and across soul sand; sneaking off, mounted again by clicking it, quick in and out.",
                    List.of("fill 8 -60 -2 9 -60 2 minecraft:stone", "fill 14 -60 5 14 -59 7 minecraft:cobweb",
                            "fill 4 -61 -7 20 -61 -5 minecraft:soul_sand"),
                    start(0.5, -60, -9.5, -90, 0), (ctx, server) -> {
                        server.runCommand("summon minecraft:horse 2.5 -60 0.5 " + HORSE);
                        ctx.waitTicks(10);
                        ride(ctx, server, "minecraft:horse");
                        hold(ctx, 30, W);
                        hold(ctx, 20, W, SPRINT);
                        ctx.waitTicks(20);
                        for (int charge : new int[] {1, 5, 9, 10, 20}) {
                            hold(ctx, charge, JUMP);
                            ctx.waitTicks(30);
                        }
                        ctx.getInput().holdKey(W);
                        ctx.waitTicks(5);
                        hold(ctx, 8, JUMP);
                        ctx.waitTicks(20);
                        ctx.getInput().releaseKey(W);
                        ctx.waitTicks(10);
                        hold(ctx, 15, W, S);
                        hold(ctx, 15, A, D);
                        hold(ctx, 20, A);
                        ctx.getInput().lookAt(180, 0);
                        hold(ctx, 25, W);
                        ctx.waitTicks(10);
                        server.runCommand("tp @e[type=minecraft:horse] 10.5 -60 6.5 -90 0");
                        ctx.waitTicks(10);
                        ctx.getInput().lookAt(-90, 0);
                        hold(ctx, 40, W);
                        ctx.waitTicks(10);
                        server.runCommand("tp @e[type=minecraft:horse] 2.5 -60 -6 -90 0");
                        ctx.waitTicks(10);
                        hold(ctx, 40, W);
                        ctx.waitTicks(10);
                        hold(ctx, 3, SNEAK);
                        ctx.waitTicks(20);
                        server.runCommand("tp @e[type=minecraft:horse] 2.5 -60 0.5 -90 0");
                        aim(ctx, server, 2.5, -60, 3.5, 2.5, -59.0, 0.5);
                        ctx.waitTicks(6);
                        use(ctx);
                        ctx.waitTicks(10);
                        hold(ctx, 10, W);
                        hold(ctx, 1, SNEAK);
                        ctx.waitTicks(5);
                        ride(ctx, server, "minecraft:horse");
                        hold(ctx, 1, SNEAK);
                        ctx.waitTicks(20);
                        gone(ctx, server, "minecraft:horse");
                    }),
            new Scenario("donkey_ride",
                    "A tame saddled donkey with a chest (speed 0.175, jump strength 0.5): walking, jumps charged for 3 and 12 ticks, a jump while walking up onto a block, into a 1-deep pool and out; sneaking off.",
                    List.of("fill 10 -60 -2 11 -60 2 minecraft:stone", "fill 4 -61 5 12 -61 8 minecraft:water"),
                    start(0.5, -60, -9.5, -90, 0), (ctx, server) -> {
                        server.runCommand("summon minecraft:donkey 2.5 -60 0.5 " + DONKEY);
                        ctx.waitTicks(10);
                        ride(ctx, server, "minecraft:donkey");
                        hold(ctx, 20, W);
                        hold(ctx, 3, JUMP);
                        ctx.waitTicks(25);
                        hold(ctx, 12, JUMP);
                        ctx.waitTicks(30);
                        ctx.getInput().holdKey(W);
                        ctx.waitTicks(15);
                        hold(ctx, 6, JUMP);
                        ctx.waitTicks(20);
                        ctx.getInput().releaseKey(W);
                        server.runCommand("tp @e[type=minecraft:donkey] 2.5 -60 6.5 -90 0");
                        ctx.waitTicks(10);
                        hold(ctx, 60, W);
                        ctx.waitTicks(10);
                        hold(ctx, 3, SNEAK);
                        ctx.waitTicks(20);
                        gone(ctx, server, "minecraft:donkey");
                    }),
            new Scenario("happy_ghast",
                    "A harnessed happy ghast: rising with jump, flying forward looking level, up and down, backward, strafing, forward and back together, rising while flying forward, turning to look south and flying on (the ghast turns a little each tick), drifting to a stop; sneaking off onto its top.",
                    List.of(),
                    start(0.5, -60, -9.5, -90, 0), (ctx, server) -> {
                        server.runCommand("summon minecraft:happy_ghast 4 -59 0.5 " + GHAST);
                        ctx.waitTicks(10);
                        ride(ctx, server, "minecraft:happy_ghast");
                        ctx.waitTicks(10);
                        hold(ctx, 20, JUMP);
                        hold(ctx, 30, W);
                        ctx.getInput().lookAt(-90, -30);
                        hold(ctx, 20, W);
                        ctx.getInput().lookAt(-90, 40);
                        hold(ctx, 20, W);
                        ctx.getInput().lookAt(-90, 0);
                        hold(ctx, 15, S);
                        hold(ctx, 15, A);
                        hold(ctx, 15, D);
                        hold(ctx, 10, W, S);
                        hold(ctx, 10, W, JUMP);
                        // Turning: the ghast follows the look a little each tick.
                        ctx.getInput().lookAt(0, 0);
                        hold(ctx, 25, W);
                        ctx.waitTicks(40);
                        hold(ctx, 3, SNEAK);
                        ctx.waitTicks(20);
                        gone(ctx, server, "minecraft:happy_ghast");
                    }),
            new Scenario("happy_ghast_ai",
                    "A harnessed happy ghast with its AI on, left to drift for 3 seconds, then mounted: rising, flying forward, sneaking off.",
                    List.of(),
                    start(0.5, -60, -9.5, -90, 0), (ctx, server) -> {
                        server.runCommand("summon minecraft:happy_ghast 4 -59 0.5 " + GHAST_AI);
                        ctx.waitTicks(60);
                        ride(ctx, server, "minecraft:happy_ghast");
                        hold(ctx, 15, JUMP);
                        hold(ctx, 30, W);
                        ctx.waitTicks(20);
                        hold(ctx, 3, SNEAK);
                        ctx.waitTicks(20);
                        gone(ctx, server, "minecraft:happy_ghast");
                    }),
            new Scenario("riding_death",
                    "Dying while paddling a boat with forward held, respawning; dying while riding a horse forward, respawning.",
                    List.of(),
                    start(0.5, -60, -9.5, -90, 0), (ctx, server) -> {
                        server.runCommand("summon minecraft:oak_boat 2.5 -60 0.5 {Rotation:[-90f,0f]}");
                        ctx.waitTicks(10);
                        ride(ctx, server, "minecraft:oak_boat");
                        ctx.getInput().holdKey(W);
                        ctx.waitTicks(15);
                        server.runCommand("kill @p");
                        ctx.waitTicks(5);
                        ctx.getInput().releaseKey(W);
                        respawn(ctx);
                        gone(ctx, server, "minecraft:oak_boat");
                        server.runCommand("tp @p 0.5 -60 -9.5 -90 0");
                        ctx.waitTicks(4);
                        server.runCommand("summon minecraft:horse 2.5 -60 0.5 " + HORSE);
                        ctx.waitTicks(10);
                        ride(ctx, server, "minecraft:horse");
                        ctx.getInput().holdKey(W);
                        ctx.waitTicks(15);
                        server.runCommand("kill @p");
                        ctx.waitTicks(5);
                        ctx.getInput().releaseKey(W);
                        respawn(ctx);
                        gone(ctx, server, "minecraft:horse");
                    }),
            new Scenario("camel_ride",
                    "A tame saddled camel (speed 0.09, jump strength 0.42), sitting when mounted: the player looks aside (it keeps its heading) and presses jump (no charge while it sits); forward held gets it up (52 ticks, the rider rising with it) and walking; sprinting; a dash charged for 10 ticks, jump held again early and late in the cooldown; turning back, a sprinting dash charged for 6 ticks; sat down while walking (40 ticks, forward held meanwhile) and got up again with forward; a charged jump let go while it falls from 3 blocks up; in a 1-deep pool, made to sit there (it gets up at once), out over the edge; sneaking off.",
                    List.of("fill 4 -61 6 14 -61 10 minecraft:water"),
                    start(0.5, -60, -9.5, -90, 0), (ctx, server) -> {
                        server.runCommand("summon minecraft:camel -5.5 -60 -6.5 " + CAMEL);
                        ctx.waitTicks(10);
                        ride(ctx, server, "minecraft:camel");
                        ctx.waitTicks(5);
                        ctx.getInput().lookAt(-60, 0);
                        ctx.waitTicks(5);
                        hold(ctx, 5, JUMP);
                        ctx.waitTicks(5);
                        ctx.getInput().lookAt(-90, 0);
                        ctx.waitTicks(3);
                        hold(ctx, 70, W);
                        hold(ctx, 20, W, SPRINT);
                        ctx.waitTicks(10);
                        hold(ctx, 10, JUMP);
                        ctx.waitTicks(10);
                        hold(ctx, 5, JUMP);
                        ctx.waitTicks(25);
                        hold(ctx, 4, JUMP);
                        ctx.waitTicks(20);
                        ctx.getInput().lookAt(90, 0);
                        ctx.getInput().holdKey(W);
                        ctx.getInput().holdKey(SPRINT);
                        ctx.waitTicks(15);
                        hold(ctx, 6, JUMP);
                        ctx.waitTicks(10);
                        ctx.getInput().releaseKey(SPRINT);
                        ctx.getInput().releaseKey(W);
                        ctx.waitTicks(20);
                        ctx.getInput().holdKey(W);
                        ctx.waitTicks(10);
                        server.runCommand(SIT_CAMEL);
                        ctx.waitTicks(40);
                        ctx.getInput().releaseKey(W);
                        ctx.waitTicks(10);
                        hold(ctx, 70, W);
                        ctx.getInput().holdKey(JUMP);
                        ctx.waitTicks(4);
                        server.runCommand("execute as @e[type=minecraft:camel] at @s run tp @s ~ ~3 ~");
                        ctx.waitTicks(3);
                        ctx.getInput().releaseKey(JUMP);
                        ctx.waitTicks(30);
                        server.runCommand("tp @e[type=minecraft:camel] 6.5 -60 8 -90 0");
                        ctx.waitTicks(10);
                        ctx.getInput().lookAt(-90, 0);
                        hold(ctx, 10, W);
                        server.runCommand(SIT_CAMEL);
                        ctx.waitTicks(10);
                        hold(ctx, 45, W);
                        ctx.waitTicks(10);
                        hold(ctx, 3, SNEAK);
                        ctx.waitTicks(20);
                        gone(ctx, server, "minecraft:camel");
                    }),
            new Scenario("pig_ride",
                    "A saddled pig ridden with a carrot on a stick held: it walks where the player looks, up a 1-block step and down again; boosted by using the stick, turning south, then north; the stick put away (it is the server's again) and taken out; sneaking off, mounted again by clicking it with an empty hand, the stick taken out; sneaking off.",
                    List.of("item replace entity @a hotbar.0 with minecraft:carrot_on_a_stick",
                            "fill 6 -60 -9 6 -60 -4 minecraft:stone"),
                    start(0.5, -60, -9.5, -90, 0), (ctx, server) -> {
                        slot(ctx, 0);
                        server.runCommand("summon minecraft:pig 0.5 -60 -6.5 " + SADDLED);
                        ctx.waitTicks(10);
                        ride(ctx, server, "minecraft:pig");
                        ctx.waitTicks(60);
                        ctx.getInput().lookAt(90, 0);
                        ctx.waitTicks(40);
                        ctx.getInput().lookAt(0, 0);
                        ctx.waitTicks(4);
                        use(ctx);
                        ctx.waitTicks(25);
                        ctx.getInput().lookAt(180, 20);
                        ctx.waitTicks(20);
                        ctx.getInput().lookAt(-90, 0);
                        ctx.waitTicks(15);
                        slot(ctx, 1);
                        ctx.waitTicks(20);
                        slot(ctx, 0);
                        ctx.getInput().lookAt(90, 0);
                        ctx.waitTicks(20);
                        hold(ctx, 3, SNEAK);
                        ctx.waitTicks(10);
                        slot(ctx, 1);
                        server.runCommand("tp @e[type=minecraft:pig] 2.5 -60 0.5 -90 0");
                        aim(ctx, server, 2.5, -60, 3.5, 2.5, -59.6, 0.5);
                        ctx.waitTicks(6);
                        use(ctx);
                        ctx.waitTicks(10);
                        slot(ctx, 0);
                        ctx.getInput().lookAt(-90, 0);
                        ctx.waitTicks(30);
                        hold(ctx, 3, SNEAK);
                        ctx.waitTicks(20);
                        gone(ctx, server, "minecraft:pig");
                    }),
            new Scenario("strider_ride",
                    "A saddled strider (its AI on) ridden with a warped fungus on a stick held: cold and slow on stone, onto a 2-deep lava pool's surface (warm), boosted by using the stick, turning about on the lava; pushed under the surface (it rises), back west onto the stone; sneaking off.",
                    List.of("item replace entity @a hotbar.0 with minecraft:warped_fungus_on_a_stick",
                            "fill 6 -62 -5 16 -61 5 minecraft:lava", UNHURT,
                            "effect give @a minecraft:fire_resistance infinite 0 true"),
                    start(0.5, -60, -9.5, -90, 0), (ctx, server) -> {
                        slot(ctx, 0);
                        server.runCommand("summon minecraft:strider 1.5 -60 0.5 " + STRIDER);
                        ctx.waitTicks(1);
                        ride(ctx, server, "minecraft:strider");
                        ctx.getInput().lookAt(-90, 0);
                        waitUntilEastOf(ctx, 8.5, 120);
                        ctx.waitTicks(10);
                        use(ctx);
                        ctx.getInput().lookAt(0, 0);
                        ctx.waitTicks(10);
                        ctx.getInput().lookAt(90, 0);
                        ctx.waitTicks(15);
                        ctx.getInput().lookAt(180, 0);
                        ctx.waitTicks(15);
                        ctx.getInput().lookAt(-90, 0);
                        ctx.waitTicks(10);
                        server.runCommand("tp @e[type=minecraft:strider] 11.5 -62 0.5 90 0");
                        ctx.waitTicks(5);
                        ctx.getInput().lookAt(90, 0);
                        waitUntilWestOf(ctx, 3.5);
                        ctx.waitTicks(10);
                        hold(ctx, 3, SNEAK);
                        ctx.waitTicks(20);
                        gone(ctx, server, "minecraft:strider");
                    }),
            new Scenario("nautilus_ride",
                    "An untamed saddled nautilus ridden with forward held (it isn't the player's to steer); then a tame saddled nautilus in a glass tank of water: swimming forward level, up and down along the look, backward, strafing; a dash charged for 10 ticks into the wall, jump held during the cooldown, a short dash; left in a channel of flowing water (the current doesn't push it); on land: walking and a dash; sneaking off.",
                    List.of("fill 2 -60 -6 22 -53 -6 minecraft:glass", "fill 2 -60 6 22 -53 6 minecraft:glass",
                            "fill 2 -60 -5 2 -53 5 minecraft:glass", "fill 22 -60 -5 22 -53 5 minecraft:glass",
                            "fill 3 -60 -5 21 -54 5 minecraft:water",
                            "fill 23 -60 8 31 -60 8 minecraft:stone", "fill 23 -60 10 31 -60 10 minecraft:stone",
                            "setblock 22 -60 9 minecraft:stone",
                            "effect give @a minecraft:water_breathing infinite 0 true"),
                    start(0.5, -60, -9.5, -90, 0), (ctx, server) -> {
                        server.runCommand("setblock 31 -60 9 minecraft:water");
                        server.runCommand("summon minecraft:nautilus 6.5 -57 0.5 " + SADDLED);
                        ctx.waitTicks(10);
                        ride(ctx, server, "minecraft:nautilus");
                        hold(ctx, 20, W);
                        hold(ctx, 3, SNEAK);
                        ctx.waitTicks(10);
                        gone(ctx, server, "minecraft:nautilus");
                        server.runCommand("summon minecraft:nautilus 6.5 -57 0.5 {" + owner(ctx) + "," + SADDLED.substring(1));
                        ctx.waitTicks(10);
                        ride(ctx, server, "minecraft:nautilus");
                        ctx.waitTicks(10);
                        hold(ctx, 15, W);
                        ctx.getInput().lookAt(-90, -40);
                        hold(ctx, 10, W);
                        ctx.getInput().lookAt(-90, 40);
                        hold(ctx, 10, W);
                        ctx.getInput().lookAt(90, 0);
                        hold(ctx, 10, S);
                        hold(ctx, 10, A);
                        hold(ctx, 10, D);
                        ctx.waitTicks(10);
                        hold(ctx, 10, JUMP);
                        ctx.waitTicks(10);
                        hold(ctx, 3, JUMP);
                        ctx.waitTicks(40);
                        ctx.getInput().lookAt(-90, -10);
                        hold(ctx, 4, JUMP);
                        ctx.waitTicks(30);
                        server.runCommand("tp @e[type=minecraft:nautilus] 27.5 -60 9.5 90 0");
                        ctx.waitTicks(30);
                        server.runCommand("tp @e[type=minecraft:nautilus] 26.5 -60 0.5 90 0");
                        ctx.waitTicks(10);
                        ctx.getInput().lookAt(90, 0);
                        hold(ctx, 20, W);
                        ctx.waitTicks(40);
                        hold(ctx, 10, JUMP);
                        ctx.waitTicks(30);
                        hold(ctx, 3, SNEAK);
                        ctx.waitTicks(20);
                        gone(ctx, server, "minecraft:nautilus");
                    }),
            new Scenario("minecart_ride",
                    "A minecart on a straight rail with three powered rails: pushed along by holding forward (the server moves it), up to a stone block at the end, back with forward held facing west; sneaking off; then a llama mounted with /ride (the rider turned to face as it does); sneaking off.",
                    List.of("fill 0 -60 0 20 -60 0 minecraft:rail[shape=east_west]",
                            "fill 8 -61 0 10 -61 0 minecraft:redstone_block",
                            "fill 8 -60 0 10 -60 0 minecraft:powered_rail[shape=east_west,powered=true]",
                            "setblock 21 -60 0 minecraft:stone", "setblock -1 -60 0 minecraft:stone"),
                    start(0.5, -60, -2.5, -90, 0), (ctx, server) -> {
                        server.runCommand("summon minecraft:minecart 1.5 -60 0.5");
                        ctx.waitTicks(10);
                        ride(ctx, server, "minecraft:minecart");
                        ctx.getInput().lookAt(-90, 0);
                        ctx.waitTicks(5);
                        hold(ctx, 30, W);
                        ctx.waitTicks(60);
                        ctx.getInput().lookAt(90, 0);
                        hold(ctx, 40, W);
                        ctx.waitTicks(40);
                        hold(ctx, 3, SNEAK);
                        ctx.waitTicks(10);
                        gone(ctx, server, "minecraft:minecart");
                        server.runCommand("summon minecraft:llama 2.5 -60 -6.5 {NoAI:1b,PersistenceRequired:1b,Silent:1b,Rotation:[45f,10f]}");
                        ctx.waitTicks(10);
                        ride(ctx, server, "minecraft:llama");
                        ctx.waitTicks(10);
                        hold(ctx, 3, SNEAK);
                        ctx.waitTicks(20);
                        gone(ctx, server, "minecraft:llama");
                    }),
            new Scenario("horse_promoted",
                    "Riding a tame horse with no saddle (a passenger, not its rider) with forward held; the horse is saddled while forward is still held, and the player takes control; sneaking off.",
                    List.of(),
                    start(0.5, -60, -9.5, -90, 0), (ctx, server) -> {
                        server.runCommand("summon minecraft:horse 2.5 -60 0.5 " + BARE_HORSE);
                        ctx.waitTicks(10);
                        ride(ctx, server, "minecraft:horse");
                        ctx.getInput().holdKey(W);
                        ctx.waitTicks(20);
                        server.runCommand("item replace entity @e[type=minecraft:horse] saddle with minecraft:saddle");
                        ctx.waitTicks(30);
                        ctx.getInput().releaseKey(W);
                        ctx.waitTicks(10);
                        hold(ctx, 3, SNEAK);
                        ctx.waitTicks(20);
                        gone(ctx, server, "minecraft:horse");
                    }));
}
