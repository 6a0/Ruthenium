package net.ruthenium.gametest;

/**
 * Wall time of the client's ticks and of their entity and block entity loops, from one {@code System.nanoTime}
 * pair per call ({@code TickProbeMixin}), so unlike the light profiler it adds nothing per entity or block entity
 * (vanilla pushes a profiler section for each, which costs more when a profiler is active). Render thread only.
 */
public final class TickProbe {
    static long tickNanos;
    static long entityNanos;
    static long blockEntityNanos;
    static int ticks;

    public static long start;
    public static long sectionStart;

    private TickProbe() {
    }

    public static void endTick(long nanos) {
        tickNanos += nanos;
        ticks++;
    }

    public static void addEntities(long nanos) {
        entityNanos += nanos;
    }

    public static void addBlockEntities(long nanos) {
        blockEntityNanos += nanos;
    }

    static void reset() {
        tickNanos = entityNanos = blockEntityNanos = 0;
        ticks = 0;
    }

    static String report() {
        int n = Math.max(1, ticks);
        return "ticks %d: %.3f ms per tick, entities %.3f, block entities %.3f".formatted(ticks, tickNanos / 1.0e6 / n,
                entityNanos / 1.0e6 / n, blockEntityNanos / 1.0e6 / n);
    }
}
