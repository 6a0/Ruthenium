package dev.ruthenium.bench;

import com.google.gson.JsonArray;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParser;
import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.List;

/**
 * A tick-keyed route, read from the file cargo xtask bench dedicated writes for this client and for
 * Ruthenium alike: where the player starts and what it does on every client tick from then on.
 *
 * <p>Tick 0 is the first tick after the start conditions have held for a second. Before tick n runs,
 * its segment's turn is applied and its keys are held; after it, the position and the rotation are
 * recorded. The loaded chunks are recorded (a mark) at tick 0, every {@code markEvery} ticks while
 * the route is measured, and once more when the player and the chunks have come to rest after the
 * last tick.
 */
final class Route {
    /** A stretch of the route: the keys held during each of its ticks and the turn before each. */
    record Segment(long ticks, List<String> keys, double turnX, double turnY) {}

    final String id;
    final String name;
    final double[] position;
    final float yaw;
    final float pitch;
    final boolean flying;
    /** Begun in survival mode instead of creative. */
    final boolean survival;
    /** The chunks loaded at the start as {@code x,z;} each, sorted by x, then z. */
    final String startChunks;
    final List<Segment> segments = new ArrayList<>();
    final long warmupTicks;
    final long captureTicks;
    final long markEvery;
    /** The server's view distance, in chunks. */
    final int viewDistance;
    /** The player must not move at all (the idle and spin routes). */
    final boolean stationary;
    final long totalTicks;

    private Route(JsonObject o) {
        if (!"ruthenium-route/1".equals(o.get("format").getAsString())) {
            throw new IllegalArgumentException("unknown route format " + o.get("format"));
        }
        id = o.get("id").getAsString();
        name = o.get("name").getAsString();
        JsonObject start = o.getAsJsonObject("start");
        JsonArray p = start.getAsJsonArray("position");
        position = new double[] {p.get(0).getAsDouble(), p.get(1).getAsDouble(), p.get(2).getAsDouble()};
        yaw = start.get("yaw").getAsFloat();
        pitch = start.get("pitch").getAsFloat();
        flying = start.get("flying").getAsBoolean();
        survival = start.has("survival") && start.get("survival").getAsBoolean();
        List<int[]> chunks = new ArrayList<>();
        for (JsonElement c : start.getAsJsonArray("chunks")) {
            JsonArray xz = c.getAsJsonArray();
            chunks.add(new int[] {xz.get(0).getAsInt(), xz.get(1).getAsInt()});
        }
        chunks.sort((a, b) -> a[0] != b[0] ? Integer.compare(a[0], b[0]) : Integer.compare(a[1], b[1]));
        StringBuilder text = new StringBuilder();
        for (int[] c : chunks) {
            text.append(c[0]).append(',').append(c[1]).append(';');
        }
        startChunks = text.toString();
        long total = 0;
        for (JsonElement e : o.getAsJsonArray("segments")) {
            JsonObject s = e.getAsJsonObject();
            List<String> keys = new ArrayList<>();
            for (JsonElement k : s.getAsJsonArray("keys")) {
                keys.add(k.getAsString());
            }
            JsonArray turn = s.getAsJsonArray("turn");
            Segment segment = new Segment(s.get("ticks").getAsLong(), keys, turn.get(0).getAsDouble(), turn.get(1).getAsDouble());
            segments.add(segment);
            total += segment.ticks();
        }
        totalTicks = total;
        warmupTicks = o.get("warmupTicks").getAsLong();
        captureTicks = o.get("captureTicks").getAsLong();
        markEvery = Math.max(1, o.get("markEvery").getAsLong());
        viewDistance = o.get("viewDistance").getAsInt();
        stationary = o.get("stationary").getAsBoolean();
        if (totalTicks < warmupTicks + captureTicks) {
            throw new IllegalArgumentException("the route's segments end before the capture does");
        }
    }

    static Route load(Path file) {
        try {
            return new Route(JsonParser.parseString(Files.readString(file)).getAsJsonObject());
        } catch (IOException e) {
            throw new IllegalStateException("reading " + file, e);
        }
    }

    /** The segment {@code tick} is in, or null after the last one. */
    Segment segment(long tick) {
        long first = 0;
        for (Segment s : segments) {
            if (tick < first + s.ticks()) {
                return s;
            }
            first += s.ticks();
        }
        return null;
    }
}
