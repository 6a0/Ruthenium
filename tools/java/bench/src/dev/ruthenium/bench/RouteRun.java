package dev.ruthenium.bench;

import com.google.gson.JsonArray;
import com.google.gson.JsonNull;
import com.google.gson.JsonObject;
import java.util.ArrayList;
import java.util.List;
import net.minecraft.client.KeyMapping;
import net.minecraft.client.Minecraft;
import net.minecraft.client.player.LocalPlayer;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.chunk.status.ChunkStatus;

/**
 * Plays a {@link Route} on a server the benchmark controller has prepared: called at the end of
 * every client tick, it waits for the route's start, gives each tick its input, and records the
 * position after each tick and the loaded chunks at the marks. The record is the same one {@code
 * ruthenium connect --route} writes, with the same two checksums, so the two clients can be compared.
 */
final class RouteRun {
    /** Ticks the start conditions must hold, and ticks of rest before the end is recorded. */
    private static final int START_TICKS = 20;
    private static final int SETTLE_TICKS = 40;
    /** How long reaching the start and coming to rest may take, in ticks. */
    private static final int START_LIMIT = 120 * 20;
    private static final int SETTLE_LIMIT = 30 * 20;
    /** Chunks are looked for this far beyond the view distance: the client keeps none farther out. */
    private static final int SCAN_MARGIN = 4;

    private enum State { WAITING, RUNNING, SETTLING, DONE }

    private final Route route;
    private State state = State.WAITING;
    private String error;
    private String waitingFor = "the server";
    private boolean rendered;
    private int waited;
    private int ready;
    private long done;
    private float yaw;
    private float pitch;
    private boolean capturing;
    /** Where the player was when the level had loaded (the controller may have moved it by then). */
    private double[] joinPosition;
    private double[] endPosition;
    private double[] restPosition;
    private String restChunks;
    private int rested;
    private int settling;
    private String startChunks = "";
    private String endChunks;
    private final List<String> ticks = new ArrayList<>();
    private final JsonArray marks = new JsonArray();
    private final JsonObject markers = new JsonObject();

    RouteRun(Route route) {
        this.route = route;
        this.yaw = route.yaw;
        this.pitch = route.pitch;
    }

    /** Whether frame intervals are being measured: from the end of the warm-up ticks for the capture ticks. */
    boolean capturing() {
        return capturing;
    }

    boolean finished() {
        return state == State.DONE;
    }

    /** The frame hook says whether every queued section has been built for a while. */
    void setRendered(boolean rendered) {
        this.rendered = rendered;
    }

    void fail(String why) {
        if (state != State.DONE) {
            error = why;
            capturing = false;
            state = State.DONE;
        }
    }

    /** The end of a client tick. */
    void tick(Minecraft mc) {
        LocalPlayer player = mc.player;
        if (state == State.DONE || player == null || mc.level == null) {
            return;
        }
        switch (state) {
            case WAITING -> {
                if (joinPosition == null) {
                    joinPosition = position(player);
                    marker("joinedEpochMs");
                }
                String chunks = loadedChunks(mc, player);
                String why = notAtStart(mc, player, chunks);
                if (why != null) {
                    waitingFor = why;
                    ready = 0;
                } else if (++ready >= START_TICKS) {
                    // A clean screen for every run: the toast a server without signed chat brings
                    // up and the chat lines of the join would outlast a short warm-up.
                    mc.gui.toastManager().clear();
                    mc.gui.hud.getChat().clearMessages(false);
                    marker("sceneReadyEpochMs");
                    marker("warmupStartEpochMs");
                    mark(0, player, chunks);
                    startChunks = chunks;
                    state = State.RUNNING;
                    apply(mc, player, 0);
                }
                if (state == State.WAITING && ++waited > START_LIMIT) {
                    fail("the route never started: " + waitingFor);
                }
            }
            case RUNNING -> {
                if (Float.floatToRawIntBits(player.getYRot()) != Float.floatToRawIntBits(yaw)
                        || Float.floatToRawIntBits(player.getXRot()) != Float.floatToRawIntBits(pitch)) {
                    fail("route tick " + done + ": the player faces " + player.getYRot() + " " + player.getXRot()
                            + ", the route says " + yaw + " " + pitch);
                    return;
                }
                if (route.stationary && !samePosition(position(player), route.position)) {
                    fail("route tick " + done + ": the player moved to " + player.getX() + " " + player.getY() + " "
                            + player.getZ() + " on a route that stands still");
                    return;
                }
                ticks.add(record(player));
                done++;
                long captureEnd = route.warmupTicks + route.captureTicks;
                if (done == route.warmupTicks) {
                    marker("captureStartEpochMs");
                    capturing = true;
                }
                if (done == captureEnd) {
                    marker("captureEndEpochMs");
                    capturing = false;
                }
                if (done % route.markEvery == 0 && done <= captureEnd) {
                    mark(done, player, loadedChunks(mc, player));
                }
                if (done == route.totalTicks) {
                    releaseKeys(mc);
                    state = State.SETTLING;
                } else {
                    apply(mc, player, done);
                }
            }
            case SETTLING -> {
                double[] now = position(player);
                String chunks = loadedChunks(mc, player);
                if (restPosition == null || !samePosition(now, restPosition) || !chunks.equals(restChunks)) {
                    restPosition = now;
                    restChunks = chunks;
                    rested = 0;
                } else if (++rested >= SETTLE_TICKS) {
                    marker("routeEndEpochMs");
                    mark(route.totalTicks, player, chunks);
                    endPosition = now;
                    endChunks = chunks;
                    state = State.DONE;
                }
                if (state == State.SETTLING && ++settling > SETTLE_LIMIT) {
                    fail("the player and the loaded chunks never came to rest after the route");
                }
            }
            case DONE -> {
            }
        }
    }

    /** Why the player isn't at the route's start yet, or null when it is. */
    private String notAtStart(Minecraft mc, LocalPlayer player, String chunks) {
        double[] p = position(player);
        if (mc.gameMode == null || mc.gameMode.getPlayerMode() != GameType.CREATIVE) {
            return "the game mode is not creative";
        } else if (!samePosition(p, route.position)) {
            return "the player is at " + p[0] + " " + p[1] + " " + p[2] + ", the route starts at " + route.position[0] + " "
                    + route.position[1] + " " + route.position[2];
        } else if (Float.floatToRawIntBits(player.getYRot()) != Float.floatToRawIntBits(route.yaw)
                || Float.floatToRawIntBits(player.getXRot()) != Float.floatToRawIntBits(route.pitch)) {
            return "the player faces " + player.getYRot() + " " + player.getXRot() + ", the route starts facing " + route.yaw
                    + " " + route.pitch;
        } else if (player.getAbilities().flying != route.flying) {
            return "flying is " + player.getAbilities().flying + ", the route starts with " + route.flying;
        } else if (player.onGround() == route.flying) {
            return "on the ground is " + player.onGround();
        } else if (!chunks.equals(route.startChunks)) {
            return "the loaded chunks are not the start's yet";
        } else if (!rendered) {
            return "sections are still being built";
        }
        return null;
    }

    /** Holds tick {@code tick}'s keys and turns the player by its turn, in the arithmetic of {@code Entity.turn}. */
    private void apply(Minecraft mc, LocalPlayer player, long tick) {
        Route.Segment s = route.segment(tick);
        List<String> keys = s == null ? List.of() : s.keys();
        mc.options.keyUp.setDown(keys.contains("forward"));
        mc.options.keyDown.setDown(keys.contains("backward"));
        mc.options.keyLeft.setDown(keys.contains("left"));
        mc.options.keyRight.setDown(keys.contains("right"));
        mc.options.keyJump.setDown(keys.contains("jump"));
        mc.options.keyShift.setDown(keys.contains("sneak"));
        mc.options.keySprint.setDown(keys.contains("sprint"));
        if (s != null && (s.turnX() != 0.0 || s.turnY() != 0.0)) {
            // The old rotation is left as it is, so the camera turns smoothly through the tick's frames.
            yaw = yaw + (float) s.turnX() * 0.15F;
            float turned = (pitch + (float) s.turnY() * 0.15F) % 360.0F;
            pitch = Math.min(Math.max(turned, -90.0F), 90.0F);
            player.setYRot(yaw);
            player.setXRot(pitch);
        }
    }

    private static void releaseKeys(Minecraft mc) {
        for (KeyMapping key : new KeyMapping[] {
            mc.options.keyUp, mc.options.keyDown, mc.options.keyLeft, mc.options.keyRight, mc.options.keyJump,
            mc.options.keyShift, mc.options.keySprint
        }) {
            key.setDown(false);
        }
    }

    private static double[] position(LocalPlayer player) {
        return new double[] {player.getX(), player.getY(), player.getZ()};
    }

    private static boolean samePosition(double[] a, double[] b) {
        for (int i = 0; i < 3; i++) {
            if (Double.doubleToRawLongBits(a[i]) != Double.doubleToRawLongBits(b[i])) {
                return false;
            }
        }
        return true;
    }

    /** One tick's record: the bits of x, y, z, then of the yaw and the pitch, as 64 hex digits. */
    private static String record(LocalPlayer player) {
        return String.format(
                "%016x%016x%016x%08x%08x",
                Double.doubleToRawLongBits(player.getX()),
                Double.doubleToRawLongBits(player.getY()),
                Double.doubleToRawLongBits(player.getZ()),
                Float.floatToRawIntBits(player.getYRot()),
                Float.floatToRawIntBits(player.getXRot()));
    }

    /** The loaded chunks around the player as {@code x,z;} each, sorted by x, then z. */
    private String loadedChunks(Minecraft mc, LocalPlayer player) {
        int cx = player.chunkPosition().x();
        int cz = player.chunkPosition().z();
        int radius = route.viewDistance + SCAN_MARGIN;
        StringBuilder text = new StringBuilder();
        for (int x = cx - radius; x <= cx + radius; x++) {
            for (int z = cz - radius; z <= cz + radius; z++) {
                if (mc.level.getChunkSource().getChunk(x, z, ChunkStatus.FULL, false) != null) {
                    text.append(x).append(',').append(z).append(';');
                }
            }
        }
        return text.toString();
    }

    private void marker(String name) {
        markers.addProperty(name, System.currentTimeMillis());
    }

    private void mark(long tick, LocalPlayer player, String chunks) {
        JsonObject m = new JsonObject();
        m.addProperty("tick", tick);
        m.addProperty("epochMs", System.currentTimeMillis());
        JsonArray p = new JsonArray();
        for (double v : position(player)) {
            p.add(v);
        }
        m.add("position", p);
        JsonArray center = new JsonArray();
        center.add(player.chunkPosition().x());
        center.add(player.chunkPosition().z());
        m.add("center", center);
        JsonArray list = new JsonArray();
        for (String c : chunks.split(";")) {
            if (!c.isEmpty()) {
                String[] xz = c.split(",");
                JsonArray pair = new JsonArray();
                pair.add(Integer.parseInt(xz[0]));
                pair.add(Integer.parseInt(xz[1]));
                list.add(pair);
            }
        }
        m.add("chunks", list);
        marks.add(m);
    }

    /** FNV-1a, 64 bits, as 16 hex digits. */
    private static String fnv1a(String text) {
        long hash = 0xcbf29ce484222325L;
        for (byte b : text.getBytes(java.nio.charset.StandardCharsets.UTF_8)) {
            hash = (hash ^ (b & 0xff)) * 0x100000001b3L;
        }
        return String.format("%016x", hash);
    }

    private static JsonArray array(double[] values) {
        JsonArray a = new JsonArray();
        for (double v : values) {
            a.add(v);
        }
        return a;
    }

    JsonObject toJson() {
        JsonObject o = new JsonObject();
        o.addProperty("format", "ruthenium-route-run/1");
        o.addProperty("client", "vanilla");
        JsonObject r = new JsonObject();
        r.addProperty("id", route.id);
        r.addProperty("name", route.name);
        o.add("route", r);
        o.add("joinPosition", joinPosition == null ? JsonNull.INSTANCE : array(joinPosition));
        o.add("endPosition", endPosition == null ? JsonNull.INSTANCE : array(endPosition));
        JsonArray records = new JsonArray();
        for (String t : ticks) {
            records.add(t);
        }
        o.add("ticks", records);
        o.add("marks", marks);
        o.add("markers", markers);
        JsonObject checksums = new JsonObject();
        checksums.addProperty("positions", fnv1a(String.join("", ticks)));
        if (endChunks == null) {
            checksums.add("footprint", JsonNull.INSTANCE);
        } else {
            checksums.addProperty("footprint", fnv1a(startChunks + "|" + endChunks));
        }
        o.add("checksums", checksums);
        if (error == null) {
            o.add("error", JsonNull.INSTANCE);
        } else {
            o.addProperty("error", error);
        }
        return o;
    }
}
