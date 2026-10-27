package net.vulkanmod.gametest;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

/**
 * Wall time per entity type of the entity work on the render thread, from one {@code System.nanoTime} pair per call
 * ({@code -Pbench.entityCost=true}, mixins in {@code net.vulkanmod.gametest.probemixin}, applied only then):
 * extraction and submission per entity type, the client tick per entity type, model prepares per model class, item
 * prepares per display context and feature renderer groups per renderer class. Each pair adds ~40-60 ns to the call
 * it times, so read the costs as a ranking and confirm a gain with an A/B run without the probe. Render thread only.
 */
public final class EntityCostProbe {
    public static final boolean ENABLED = Boolean.getBoolean("vulkanmod.bench.entityCost");

    public static final Table EXTRACT = new Table("extract (per entity type)", false);
    public static final Table SUBMIT = new Table("submit (per entity type)", false);
    public static final Table TICK = new Table("tick (per entity type)", true);
    public static final Table FEATURE_GROUPS = new Table("prepare groups (per feature renderer)", false);
    public static final Table MODELS = new Table("prepare models (per model class and pipeline)", false);
    public static final Table SETUP_ANIM = new Table("setupAnim of posed models (per model class)", false);
    public static final Table MODEL_RENDER = new Table("part tree of posed models (per model class)", false);
    public static final Table ITEMS = new Table("prepare items (per display context, extra = quads)", false);
    private static final List<Table> TABLES = List.of(EXTRACT, SUBMIT, FEATURE_GROUPS, MODELS, SETUP_ANIM, MODEL_RENDER, ITEMS, TICK);

    private EntityCostProbe() {
    }

    public static final class Table {
        final String name;
        final boolean perTick;
        final Map<Object, long[]> entries = new HashMap<>();

        Table(String name, boolean perTick) {
            this.name = name;
            this.perTick = perTick;
        }

        public void add(Object key, long nanos, long extra) {
            long[] entry = this.entries.get(key);
            if (entry == null) {
                entry = new long[3];
                this.entries.put(key, entry);
            }
            entry[0] += nanos;
            entry[1]++;
            entry[2] += extra;
        }
    }

    static void reset() {
        for (Table table : TABLES) {
            table.entries.clear();
        }
    }

    static String report(String phase, int frames, int ticks) {
        StringBuilder out = new StringBuilder("Entity cost probe of ").append(phase).append(": ").append(frames)
                .append(" frames, ").append(ticks).append(" ticks\n");
        for (Table table : TABLES) {
            int n = Math.max(1, table.perTick ? ticks : frames);
            List<Map.Entry<Object, long[]>> entries = new ArrayList<>(table.entries.entrySet());
            entries.sort((a, b) -> Long.compare(b.getValue()[0], a.getValue()[0]));
            long total = entries.stream().mapToLong(e -> e.getValue()[0]).sum();
            out.append("%s: %.3f ms per %s\n".formatted(table.name, total / 1.0e6 / n, table.perTick ? "tick" : "frame"));
            out.append("  %-48s %10s %10s %10s %10s%n".formatted("key", "us/" + (table.perTick ? "tick" : "frame"),
                    "calls", "ns/call", "extra"));
            for (Map.Entry<Object, long[]> e : entries) {
                long[] v = e.getValue();
                out.append("  %-48s %10.1f %10.1f %10.0f %10.1f%n".formatted(name(e.getKey()), v[0] / 1.0e3 / n, (double) v[1] / n,
                        (double) v[0] / v[1], (double) v[2] / v[1]));
            }
        }
        return out.toString();
    }

    private static String name(Object key) {
        if (key instanceof List<?> list) {
            return list.stream().map(EntityCostProbe::name).reduce((a, b) -> a + " " + b).orElse("");
        }
        return key instanceof Class<?> c ? c.getName().substring(c.getName().lastIndexOf('.') + 1) : String.valueOf(key);
    }
}
