package net.ruthenium.gametest;

import com.mojang.blaze3d.platform.NativeImage;
import net.fabricmc.fabric.api.client.gametest.v1.context.ClientGameTestContext;
import net.fabricmc.fabric.api.client.gametest.v1.context.TestServerContext;
import net.fabricmc.fabric.api.client.gametest.v1.context.TestSingleplayerContext;
import net.minecraft.client.CloudStatus;
import net.minecraft.client.Minecraft;
import net.minecraft.client.entity.ClientMannequin;
import net.minecraft.client.renderer.texture.DynamicTexture;
import net.minecraft.client.renderer.texture.SkinTextureDownloader;
import net.minecraft.client.resources.SkinManager;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.util.ARGB;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.animal.parrot.Parrot;
import net.minecraft.world.entity.decoration.Mannequin;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import java.io.IOException;
import java.io.InputStream;
import java.lang.reflect.Field;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.TreeMap;
import java.util.concurrent.CompletableFuture;

/**
 * Players and holograms benchmark ({@code -Pbench.scene=players}): what a server lobby shows that the other scenes
 * don't, i.e. other players and floating text.
 * <p>
 * Players are {@code minecraft:mannequin}s: the client extracts them with an {@code AvatarRenderer} and submits them
 * through the same player renderers (by skin model) as remote players, with all of a player's layers: skin with outer
 * layers on the wide and slim models, capes, elytra, armor with trims, held items, arrows and bee stingers stuck in
 * them, shoulder parrots (set on the client entity by the harness, {@link BenchParrots}; mannequins never have any),
 * name tags with team prefixes and colours, and a line below the name (the mannequin's description, which takes the
 * place of a below-name score). Their skins are registered as {@link DynamicTexture}s by default, as downloaded skins
 * are ({@code bench.playerSkins=downloaded}; {@code resource} uses vanilla's default skin resources directly). What
 * differs from remote players: their tick ({@code ClientMannequin} is a living entity without a player's tick), and
 * they stand still.
 * <p>
 * Holograms are stacks of lines: invisible marker armor stands with visible custom names (one per line), text displays
 * (one per line) and text displays holding all lines, all facing the camera. They are static.
 * <p>
 * {@code bench.playersKind}: {@code lobby} (the default: both), {@code players} or {@code holograms}. Counts:
 * {@code bench.playerCount} (150), {@code bench.hologramCount} (120 holograms, shared out between the three kinds of
 * {@code bench.hologramKind=mixed}, or {@code stand}, {@code text}, {@code multiline}), {@code bench.hologramLines}
 * (4). The camera is a flying creative player (a spectator would see the invisible armor stands). Phases: "static"
 * facing the crowd and "pan" (360 degrees). Screenshot: the static view. {@code bench.closeShots=N} and
 * {@code bench.closeToggle=<Config field>} add frozen close-ups (players, holograms) as in the entity scene. Results
 * are labelled {@code <label>-players-<kind>}.
 */
final class PlayerBenchmark {
    private static final Logger LOGGER = LoggerFactory.getLogger("Ruthenium Player Benchmark");
    private static final int TICKS_PER_SECOND = 20;

    private static final String KIND = BenchmarkClientGameTest.prop("playersKind", "lobby");
    private static final boolean PLAYERS = KIND.equals("lobby") || KIND.equals("players");
    private static final boolean HOLOGRAMS = KIND.equals("lobby") || KIND.equals("holograms");
    private static final int PLAYER_COUNT = Integer.parseInt(BenchmarkClientGameTest.prop("playerCount", "150"));
    private static final int HOLOGRAM_COUNT = Integer.parseInt(BenchmarkClientGameTest.prop("hologramCount", "120"));
    private static final int HOLOGRAM_LINES = Integer.parseInt(BenchmarkClientGameTest.prop("hologramLines", "4"));
    private static final String HOLOGRAM_KIND = BenchmarkClientGameTest.prop("hologramKind", "mixed");
    private static final boolean DOWNLOADED_SKINS = BenchmarkClientGameTest.prop("playerSkins", "downloaded").equals("downloaded");
    /** Distinct skins: the 18 defaults (9 skins, wide and slim), then tinted copies with other outer layers */
    private static final int SKIN_VARIANTS = DOWNLOADED_SKINS ? Integer.parseInt(BenchmarkClientGameTest.prop("skinVariants", "18")) : 18;
    private static final int RENDER_DISTANCE = Integer.parseInt(BenchmarkClientGameTest.prop("playersRenderDistance", "12"));
    private static final int CLOSE_SHOTS = Integer.parseInt(BenchmarkClientGameTest.prop("closeShots", "0"));
    private static final String CLOSE_TOGGLE = BenchmarkClientGameTest.prop("closeToggle", "");

    /** The flat world's surface */
    private static final int FLOOR = -60;
    /** The camera (feet), looking along +z */
    private static final double CAMERA_X = 0.5, CAMERA_Y = FLOOR + 2.0, CAMERA_Z = -7.5;
    private static final float PITCH = 8.0F;
    /** The crowd: COLUMNS players per row, 1.5 blocks apart, rows 2 blocks apart from z = 2 */
    private static final int COLUMNS = 15;
    private static final double PLAYER_MIN_X = -10.5, PLAYER_DX = 1.5, PLAYER_MIN_Z = 2.0, PLAYER_DZ = 2.0;
    /** Holograms: 9 per row, 3 blocks apart, their lowest line 3 blocks above the floor (above the crowd's name tags) */
    private static final int HOLOGRAM_COLUMNS = 9;
    private static final double HOLOGRAM_MIN_X = -12.0, HOLOGRAM_DX = 3.0, HOLOGRAM_MIN_Z = 3.0, HOLOGRAM_DZ = 3.0;
    private static final double HOLOGRAM_BASE = 3.0, LINE_SPACING = 0.25;

    /** Close-up camera positions (x, height above the floor, z, pitch), looking along +z */
    private static final float[][] CLOSE_UPS = {
            {0.5F, 1.6F, -1.5F, 5.0F},   // the first rows of players
            {0.5F, 3.2F, 0.0F, 0.0F},    // the first rows of holograms
    };

    private static final String[] SKINS = {"alex", "ari", "efe", "kai", "makena", "noor", "steve", "sunny", "zuri"};
    private static final String[] NAME_HEADS = {"Shadow", "Craft", "Pixel", "Ender", "Blaze", "Frost", "Storm", "Lucky", "Diamond",
            "Red", "xX_", "Sky", "Night", "Iron", "Super"};
    private static final String[] NAME_TAILS = {"Miner", "King", "Wolf", "Ninja", "Fox", "Gamer", "Builder", "Hunter", "Knight",
            "Dragon", "Pro"};
    private static final String[] MAIN_HANDS = {"diamond_sword", "bow", null, "iron_pickaxe", "oak_planks", "golden_apple", "trident",
            "crossbow", null, "fishing_rod", "netherite_sword", "torch"};
    private static final String[] GAMES = {"BEDWARS", "SKYWARS", "DUELS", "BUILD BATTLE", "MURDER MYSTERY", "PARKOUR", "SURVIVAL",
            "CREATIVE", "THE PIT"};
    private static final String[] GAME_COLORS = {"gold", "aqua", "red", "light_purple", "dark_red", "green", "yellow", "blue", "white"};

    /** Entities summoned for the holograms */
    private static int hologramEntityCount;

    private PlayerBenchmark() {
    }

    static void run(ClientGameTestContext context, String label, String runId) {
        if (!PLAYERS && !HOLOGRAMS) {
            throw new AssertionError("Unknown bench.playersKind '" + KIND + "' (lobby, players or holograms)");
        }
        String sceneLabel = label + "-players-" + KIND;
        context.runOnClient(mc -> {
            mc.options.renderDistance().set(RENDER_DISTANCE);
            mc.options.cloudStatus().set(CloudStatus.OFF);
        });
        if (DOWNLOADED_SKINS) {
            registerDownloadedSkins(context);
        }

        List<PhaseResult> results = new ArrayList<>();
        Map<String, String> frameSeries = new HashMap<>();

        try (TestSingleplayerContext world = context.worldBuilder().create()) {
            TestServerContext server = world.getServer();
            server.runCommand("time set 6000");
            server.runCommand("difficulty peaceful");
            // A spectator would see the invisible armor stands of the holograms
            server.runCommand("gamemode creative @a");
            int players = PLAYERS ? buildPlayers(server) : 0;
            int lines = HOLOGRAMS ? buildHolograms(server) : 0;
            moveCamera(context, server, (float) CAMERA_X, (float) (CAMERA_Y - FLOOR), (float) CAMERA_Z, PITCH);
            context.waitTicks(TICKS_PER_SECOND);
            TerrainBenchmark.waitForVisibleCompiled(context);
            moveCamera(context, server, (float) CAMERA_X, (float) (CAMERA_Y - FLOOR), (float) CAMERA_Z, PITCH);
            context.waitTicks(2 * TICKS_PER_SECOND);

            Map<String, Integer> counts = context.computeOnClient(PlayerBenchmark::countEntities);
            LOGGER.info("Entities: {} ({} players, {} hologram lines summoned)", counts, players, lines);
            int mannequins = counts.getOrDefault("mannequin", 0);
            if (mannequins < players) {
                throw new AssertionError("Only " + mannequins + " of " + players + " players arrived: " + counts);
            }
            int hologramEntities = counts.getOrDefault("armor_stand", 0) + counts.getOrDefault("text_display", 0);
            if (hologramEntities < hologramEntityCount) {
                throw new AssertionError("Only " + hologramEntities + " of " + hologramEntityCount + " hologram entities arrived: " + counts);
            }
            if (PLAYERS) {
                int parrots = context.computeOnClient(PlayerBenchmark::setParrots);
                LOGGER.info("{} mannequins with shoulder parrots", parrots);
            }

            context.takeScreenshot(sceneLabel + "-static-" + runId);

            setStuckArrows(server);
            context.runOnClient(_ -> FrameRecorder.startPhase());
            BenchmarkClientGameTest.waitSeconds(context, BenchmarkClientGameTest.STATIC_SECONDS);
            results.add(context.computeOnClient(_ -> FrameRecorder.stopPhase("static", -1)));
            LOGGER.info(results.getLast().summaryLine());

            setStuckArrows(server);
            context.runOnClient(mc -> {
                FrameRecorder.startPhase();
                FrameRecorder.startPan(mc, 0.0F, PITCH, BenchmarkClientGameTest.PAN_SECONDS);
            });
            context.waitFor(_ -> !FrameRecorder.isPanning(), (int) (BenchmarkClientGameTest.PAN_SECONDS * TICKS_PER_SECOND * 10));
            results.add(context.computeOnClient(_ -> FrameRecorder.stopPhase("pan", -1)));
            frameSeries.put("pan", context.computeOnClient(_ -> FrameRecorder.framesCsv()));
            LOGGER.info(results.getLast().summaryLine());

            if (CLOSE_SHOTS > 0) {
                closeUps(context, server, sceneLabel, runId);
            }

            var environment = context.computeOnClient(BenchmarkClientGameTest::describeEnvironment);
            environment.addProperty("label", sceneLabel);
            environment.addProperty("playersKind", KIND);
            environment.addProperty("players", players);
            environment.addProperty("hologramLines", lines);
            environment.addProperty("hologramKind", HOLOGRAM_KIND);
            environment.addProperty("playerSkins", DOWNLOADED_SKINS ? "downloaded" : "resource");
            environment.addProperty("skinVariants", SKIN_VARIANTS);
            var entities = new com.google.gson.JsonObject();
            counts.forEach(entities::addProperty);
            environment.add("entities", entities);
            BenchmarkClientGameTest.writeResults(sceneLabel, results, environment, frameSeries);
        }
    }

    /**
     * Registers the skins through vanilla's skin downloader ({@code SkinTextureDownloader.downloadAndRegisterSkin}) from
     * local copies written into {@code bench-skins/} of the run directory, so nothing is downloaded but everything after
     * the download is a downloaded skin's path: {@code processLegacySkin} (64x32 skins converted, the base layer made
     * opaque) and a {@link DynamicTexture} registered under {@code ruthenium-gametest:textures/skins/v<variant>.png}.
     * Variants 0-17 are the default skins (wide and slim in turn); later ones ({@code bench.skinVariants}) are tinted
     * copies whose outer layer regions are each kept, cleared (alpha 0), faded below the cutout (alpha 20),
     * semi-transparent (alpha 100) or cleared on every other texel, and every tenth of them (wide ones) a legacy 64x32 skin.
     */
    private static void registerDownloadedSkins(ClientGameTestContext context) {
        Path dir = context.computeOnClient(mc -> {
            Path skins = mc.gameDirectory.toPath().resolve("bench-skins");
            try {
                Files.createDirectories(skins);
                for (int v = 0; v < SKIN_VARIANTS; v++) {
                    try (NativeImage image = skinImage(mc, v)) {
                        image.writeToFile(skins.resolve("v" + v + ".png"));
                    }
                }
            } catch (IOException e) {
                throw new AssertionError("Can't write the bench skins", e);
            }
            return skins;
        });
        SkinTextureDownloader downloader = context.computeOnClient(mc -> {
            try {
                Field field = SkinManager.class.getDeclaredField("skinTextureDownloader");
                field.setAccessible(true);
                return (SkinTextureDownloader) field.get(mc.getSkinManager());
            } catch (ReflectiveOperationException e) {
                throw new AssertionError("No skin downloader", e);
            }
        });
        List<CompletableFuture<?>> futures = new ArrayList<>();
        for (int v = 0; v < SKIN_VARIANTS; v++) {
            Identifier texture = Identifier.fromNamespaceAndPath("ruthenium-gametest", "textures/skins/v" + v + ".png");
            futures.add(downloader.downloadAndRegisterSkin(texture, dir.resolve("v" + v + ".png"), "https://bench.invalid/skins/v" + v, true));
        }
        context.waitFor(_ -> futures.stream().allMatch(CompletableFuture::isDone), 60 * TICKS_PER_SECOND);
        futures.forEach(CompletableFuture::join);
        LOGGER.info("{} skins registered through the skin downloader ({} legacy)", SKIN_VARIANTS,
                java.util.stream.IntStream.range(0, SKIN_VARIANTS).filter(PlayerBenchmark::isLegacySkin).count());
    }

    private static String skinModel(int variant) {
        return variant % 2 == 0 ? "wide" : "slim";
    }

    /** Whether a skin variant is a legacy 64x32 skin */
    private static boolean isLegacySkin(int variant) {
        return variant >= 18 && variant % 2 == 0 && variant / 2 % 5 == 4;
    }

    /** The skin file of a variant, before vanilla's processing */
    private static NativeImage skinImage(Minecraft mc, int variant) throws IOException {
        String base = SKINS[variant / 2 % SKINS.length];
        Identifier source = Identifier.withDefaultNamespace("textures/entity/player/%s/%s.png".formatted(skinModel(variant), base));
        NativeImage image;
        try (InputStream in = mc.getResourceManager().open(source)) {
            image = NativeImage.read(in);
        }
        if (image.getWidth() != 64 || image.getHeight() != 64) {
            throw new AssertionError("Unexpected skin size of " + source);
        }
        if (variant < 18) {
            return image;
        }
        int hash = variant * 0x9E3779B9;
        float r = 0.55F + (hash >>> 8 & 0xFF) / 570.0F, g = 0.55F + (hash >>> 16 & 0xFF) / 570.0F, b = 0.55F + (hash >>> 24) / 570.0F;
        for (int y = 0; y < 64; y++) {
            for (int x = 0; x < 64; x++) {
                int pixel = image.getPixel(x, y);
                image.setPixel(x, y, ARGB.color(ARGB.alpha(pixel), (int) (ARGB.red(pixel) * r), (int) (ARGB.green(pixel) * g), (int) (ARGB.blue(pixel) * b)));
            }
        }
        // Outer layer regions: hat, jacket, right sleeve, right pants, left sleeve, left pants
        int[][] regions = {{32, 0, 64, 16}, {16, 32, 40, 48}, {40, 32, 56, 48}, {0, 32, 16, 48}, {48, 48, 64, 64}, {0, 48, 16, 64}};
        int ops = Integer.rotateLeft(hash, 11) & Integer.MAX_VALUE;
        for (int[] region : regions) {
            int op = ops % 5;
            ops /= 5;
            for (int y = region[1]; y < region[3]; y++) {
                for (int x = region[0]; x < region[2]; x++) {
                    int pixel = image.getPixel(x, y);
                    int alpha = switch (op) {
                        case 1 -> 0;
                        case 2 -> 20;
                        case 3 -> 100;
                        case 4 -> (x + y) % 2 == 0 ? 0 : 255;
                        default -> ARGB.alpha(pixel);
                    };
                    image.setPixel(x, y, ARGB.color(alpha, pixel));
                }
            }
        }
        if (isLegacySkin(variant)) {
            NativeImage legacy = new NativeImage(64, 32, false);
            for (int y = 0; y < 32; y++) {
                for (int x = 0; x < 64; x++) {
                    legacy.setPixel(x, y, image.getPixel(x, y));
                }
            }
            image.close();
            return legacy;
        }
        return image;
    }

    private static int buildPlayers(TestServerContext server) {
        server.runCommand("team add vip");
        server.runCommand("team modify vip prefix {text:\"[VIP] \",color:\"green\"}");
        server.runCommand("team modify vip color green");
        server.runCommand("team add mvp");
        server.runCommand("team modify mvp prefix {text:\"[MVP+] \",color:\"aqua\"}");
        server.runCommand("team modify mvp color aqua");
        server.runCommand("team add admin");
        server.runCommand("team modify admin prefix {text:\"[ADMIN] \",color:\"red\",bold:true}");
        server.runCommand("team modify admin color red");

        for (int i = 0; i < PLAYER_COUNT; i++) {
            double x = PLAYER_MIN_X + PLAYER_DX * (i % COLUMNS);
            double z = PLAYER_MIN_Z + PLAYER_DZ * (i / COLUMNS);
            int variant = i % SKIN_VARIANTS;
            String model = skinModel(variant);
            String skin = DOWNLOADED_SKINS ? "ruthenium-gametest:skins/v" + variant
                    : "minecraft:entity/player/%s/%s".formatted(model, SKINS[variant / 2 % SKINS.length]);
            boolean elytra = i % 7 == 2;
            boolean cape = i % 3 == 0 && !elytra;

            String profileNbt = "profile:{texture:\"%s\",model:\"%s\"".formatted(skin, model);
            if (cape) {
                profileNbt += ",cape:\"ruthenium-gametest:entity/cape/bench\"";
            }
            profileNbt += "}";

            StringBuilder nbt = new StringBuilder(profileNbt);
            String name = NAME_HEADS[i % NAME_HEADS.length] + NAME_TAILS[(i * 7 + 3) % NAME_TAILS.length] + (i % 3 == 1 ? Integer.toString(i) : "");
            nbt.append(",CustomName:\"").append(name).append("\",CustomNameVisible:1b");
            if (i % 2 == 0) {
                nbt.append(",description:{text:\"").append(20 - i % 7).append(" \u2764\",color:\"red\"}");
            } else {
                nbt.append(",hide_description:1b");
            }
            float yaw = 180.0F + (i * 37 % 41) - 20;
            nbt.append(",Rotation:[%sf,0f]".formatted(yaw));
            if (i % 15 == 7) {
                nbt.append(",pose:\"crouching\"");
            }
            if (i % 8 == 5) {
                nbt.append(",hidden_layers:[\"hat\",\"jacket\"]");
            } else if (i % 8 == 6) {
                nbt.append(",hidden_layers:[\"left_sleeve\",\"right_sleeve\"]");
            }
            nbt.append(',').append(equipment(i, elytra));
            String team = i % 4 == 0 ? "vip" : i % 4 == 1 ? "mvp" : i % 12 == 2 ? "admin" : null;
            if (team != null) {
                nbt.append(",Tags:[\"bench_").append(team).append("\"]");
            }
            server.runCommand("summon minecraft:mannequin %s %d %s {%s}".formatted(x, FLOOR, z, nbt));
        }
        for (String team : new String[]{"vip", "mvp", "admin"}) {
            server.runCommand("team join %s @e[type=minecraft:mannequin,tag=bench_%s]".formatted(team, team));
        }
        return PLAYER_COUNT;
    }

    private static String equipment(int i, boolean elytra) {
        List<String> slots = new ArrayList<>();
        String trim = i % 10 == 0 ? "{material:\"minecraft:gold\",pattern:\"minecraft:bolt\"}"
                : "{material:\"minecraft:diamond\",pattern:\"minecraft:sentry\"}";
        switch (i % 5) {
            case 0 -> {
                slots.add("head:" + item("iron_helmet"));
                slots.add("chest:{id:\"minecraft:iron_chestplate\",count:1,components:{\"minecraft:trim\":%s}}".formatted(trim));
                slots.add("legs:" + item("iron_leggings"));
                slots.add("feet:" + item("iron_boots"));
            }
            case 1 -> {
                slots.add("chest:{id:\"minecraft:diamond_chestplate\",count:1,components:{\"minecraft:trim\":%s}}".formatted(trim));
                slots.add("legs:{id:\"minecraft:diamond_leggings\",count:1,components:{\"minecraft:enchantments\":{\"minecraft:protection\":1}}}");
            }
            case 2 -> {
                slots.add("head:" + item("netherite_helmet"));
                slots.add("feet:" + item("netherite_boots"));
            }
            default -> {
            }
        }
        if (elytra) {
            slots.removeIf(s -> s.startsWith("chest:"));
            slots.add("chest:" + item("elytra"));
        }
        String main = MAIN_HANDS[i % MAIN_HANDS.length];
        if (main != null) {
            slots.add("mainhand:" + item(main));
        }
        if (i % 6 == 0) {
            slots.add("offhand:" + item("shield"));
        } else if (i % 6 == 3) {
            slots.add("offhand:" + item("torch"));
        }
        return "equipment:{" + String.join(",", slots) + "}";
    }

    private static String item(String id) {
        return "{id:\"minecraft:%s\",count:1}".formatted(id);
    }

    /** The player index of a mannequin from its position in the grid, or -1 */
    private static int playerIndex(Entity entity) {
        int column = (int) Math.round((entity.getX() - PLAYER_MIN_X) / PLAYER_DX);
        int row = (int) Math.round((entity.getZ() - PLAYER_MIN_Z) / PLAYER_DZ);
        if (column < 0 || column >= COLUMNS || row < 0) {
            return -1;
        }
        int index = row * COLUMNS + column;
        return index < PLAYER_COUNT ? index : -1;
    }

    /** Arrows (every fourth player) and bee stingers (every ninth), set again before each phase as they wear off */
    private static void setStuckArrows(TestServerContext server) {
        if (!PLAYERS) {
            return;
        }
        server.runOnServer(s -> {
            for (Entity entity : s.overworld().getAllEntities()) {
                if (entity instanceof Mannequin mannequin) {
                    int i = playerIndex(mannequin);
                    if (i < 0) {
                        continue;
                    }
                    mannequin.setArrowCount(i % 4 == 1 ? 1 + i % 5 : 0);
                    mannequin.setStingerCount(i % 9 == 4 ? 1 + i % 3 : 0);
                }
            }
        });
    }

    private static int setParrots(Minecraft mc) {
        Parrot.Variant[] variants = Parrot.Variant.values();
        int count = 0;
        for (Entity entity : mc.level.entitiesForRendering()) {
            if (entity instanceof ClientMannequin && entity instanceof BenchParrots parrots) {
                int i = playerIndex(entity);
                Parrot.Variant left = i >= 0 && i % 6 == 2 ? variants[i % variants.length] : null;
                Parrot.Variant right = i >= 0 && i % 10 == 7 ? variants[(i + 2) % variants.length] : null;
                parrots.ruthenium$setParrots(left, right);
                if (left != null || right != null) {
                    count++;
                }
            }
        }
        return count;
    }

    /** Builds the holograms; returns the number of lines */
    private static int buildHolograms(TestServerContext server) {
        int lines = 0;
        for (int h = 0; h < HOLOGRAM_COUNT; h++) {
            double x = HOLOGRAM_MIN_X + HOLOGRAM_DX * (h % HOLOGRAM_COLUMNS) + 0.5;
            double z = HOLOGRAM_MIN_Z + HOLOGRAM_DZ * (h / HOLOGRAM_COLUMNS) + 0.5;
            String kind = switch (HOLOGRAM_KIND) {
                case "mixed" -> new String[]{"stand", "text", "multiline"}[h % 3];
                case "stand", "text", "multiline" -> HOLOGRAM_KIND;
                default -> throw new AssertionError("Unknown bench.hologramKind '" + HOLOGRAM_KIND + "' (mixed, stand, text or multiline)");
            };
            // Lines from the top down, as hologram plugins stack them
            double top = FLOOR + HOLOGRAM_BASE + LINE_SPACING * (HOLOGRAM_LINES - 1);
            if (kind.equals("multiline")) {
                StringBuilder text = new StringBuilder("[");
                for (int l = 0; l < HOLOGRAM_LINES; l++) {
                    if (l > 0) {
                        text.append(",\"\\n\",");
                    }
                    text.append(line(h, l));
                }
                text.append(']');
                summon(server, "text_display", x, FLOOR + HOLOGRAM_BASE, z, "text:%s,billboard:\"center\"".formatted(text));
                hologramEntityCount++;
            } else {
                for (int l = 0; l < HOLOGRAM_LINES; l++) {
                    double y = top - LINE_SPACING * l;
                    if (kind.equals("stand")) {
                        summon(server, "armor_stand", x, y, z, "Invisible:1b,Marker:1b,NoGravity:1b,CustomNameVisible:1b,CustomName:" + line(h, l));
                    } else {
                        summon(server, "text_display", x, y, z, "text:%s,billboard:\"center\"".formatted(line(h, l)));
                    }
                    hologramEntityCount++;
                }
            }
            lines += HOLOGRAM_LINES;
        }
        return lines;
    }

    /** Line l of hologram h as an SNBT text component */
    private static String line(int h, int l) {
        return switch (l % 4) {
            case 0 -> "{text:\"%s\",color:\"%s\",bold:1b}".formatted(GAMES[h % GAMES.length], GAME_COLORS[h % GAME_COLORS.length]);
            case 1 -> String.format(Locale.ROOT, "[{text:\"Players: \",color:\"gray\"},{text:\"%,d\",color:\"green\"}]", 37 + h * 211 % 4000);
            case 2 -> "{text:\"Click to play!\",color:\"yellow\"}";
            default -> "{text:\"Hologram #%d line %d\",color:\"dark_gray\"}".formatted(h, l + 1);
        };
    }

    private static void summon(TestServerContext server, String type, double x, double y, double z, String nbt) {
        server.runCommand("summon minecraft:%s %s %s %s {%s}".formatted(type, x, y, z, nbt));
    }

    private static void closeUps(ClientGameTestContext context, TestServerContext server, String label, String runId) {
        server.runCommand("tick freeze");
        context.waitTicks(5);
        for (int k = 0; k < CLOSE_UPS.length; k++) {
            float[] view = CLOSE_UPS[k];
            moveCamera(context, server, view[0], view[1], view[2], view[3]);
            for (int n = 0; n < CLOSE_SHOTS; n++) {
                boolean toggled = !CLOSE_TOGGLE.isEmpty() && n % 2 == 1;
                if (toggled) {
                    toggleConfig(context);
                }
                context.takeScreenshot("%s-close%d-%d-%s".formatted(label, k, n, runId));
                if (toggled) {
                    toggleConfig(context);
                }
            }
        }
        server.runCommand("tick unfreeze");
    }

    /** Moves the flying creative camera player (x, height above the floor, z, pitch), looking along +z */
    private static void moveCamera(ClientGameTestContext context, TestServerContext server, float x, float height, float z, float pitch) {
        server.runCommand("tp @a %s %s %s 0 %s".formatted(x, FLOOR + height, z, pitch));
        context.waitTicks(5);
        context.runOnClient(mc -> {
            mc.player.getAbilities().flying = true;
            mc.player.onUpdateAbilities();
            mc.player.setDeltaMovement(0.0, 0.0, 0.0);
            mc.player.setYRot(0.0F);
            mc.player.yRotO = 0.0F;
            mc.player.setXRot(pitch);
            mc.player.xRotO = pitch;
        });
        context.waitTicks(5);
    }

    /** Flips the boolean {@code Config} field named by {@code bench.closeToggle}, by reflection (vanilla runs have no mod) */
    private static void toggleConfig(ClientGameTestContext context) {
        context.runOnClient(_ -> {
            try {
                Object config = Class.forName("net.ruthenium.Initializer").getField("CONFIG").get(null);
                var field = config.getClass().getField(CLOSE_TOGGLE);
                field.setBoolean(config, !field.getBoolean(config));
            } catch (ReflectiveOperationException e) {
                throw new AssertionError("Can't toggle " + CLOSE_TOGGLE, e);
            }
        });
    }

    private static Map<String, Integer> countEntities(Minecraft mc) {
        Map<String, Integer> counts = new TreeMap<>();
        for (Entity entity : mc.level.entitiesForRendering()) {
            counts.merge(BuiltInRegistries.ENTITY_TYPE.getKey(entity.getType()).getPath(), 1, Integer::sum);
        }
        return counts;
    }
}
