package net.rubidium.gametest;

import net.minecraft.util.profiling.ProfilerFiller;
import net.minecraft.util.profiling.metrics.MetricCategory;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Locale;
import java.util.function.Supplier;

/**
 * A cheap replacement for vanilla's section profiler ({@code -Pbench.lightProfile=DEPTH}). It times vanilla's own
 * profiler sections with {@code System.nanoTime} down to DEPTH levels below the root and only counts deeper pushes,
 * so per entity and per block entity sections don't inflate their parents the way vanilla's {@code ActiveProfiler}
 * does (string paths and a map lookup per push). {@code incrementCounter} amounts are summed per section.
 * <p>
 * There is no CPU time column on purpose: Windows charges thread CPU time in scheduler ticks, so per section CPU
 * times came out larger than the wall time (vanilla's "submit": 0.58 ms CPU in 0.23 ms wall). Waiting is visible as
 * its own section instead (e.g. {@code awaitGpu} from {@code VulkanCommandEncoderProfileMixin}).
 * <p>
 * The game installs it per loop iteration (one frame plus its ticks) through {@code MinecraftProfilerMixin}; only
 * the render thread uses it.
 * <p>
 * Each phase also gets a split by frame kind: per section, ms per frame over the loop iterations that ran a client
 * tick and over those that didn't (the tick runs every ~12th frame at ~250 fps and sets the p95/p99 low), so work a
 * tick causes outside its own section shows up as a difference between the two columns.
 */
public final class LightProfiler implements ProfilerFiller {
    public static final int MAX_DEPTH = Integer.parseInt(System.getProperty("rubidium.bench.lightProfile", "0"));
    public static final LightProfiler INSTANCE = new LightProfiler();

    private static final class Node {
        final String name;
        final Node parent;
        final HashMap<String, Node> children = new HashMap<>();
        final List<Node> order = new ArrayList<>();
        final HashMap<String, long[]> counters = new HashMap<>();
        long nanos;
        long calls;
        /** This loop iteration's time, committed at its end to the tick or no-tick totals */
        long pending;
        long tickNanos;
        long otherNanos;
        boolean touched;
        /** Pushes below the timed depth, attributed to their nearest timed ancestor */
        long deepPushes;

        Node(String name, Node parent) {
            this.name = name;
            this.parent = parent;
        }

        Node child(String childName) {
            Node child = this.children.get(childName);
            if (child == null) {
                child = new Node(childName, this);
                this.children.put(childName, child);
                this.order.add(child);
            }
            return child;
        }

        void reset() {
            this.nanos = 0;
            this.counters.clear();
            this.calls = 0;
            this.deepPushes = 0;
            this.pending = 0;
            this.tickNanos = 0;
            this.otherNanos = 0;
            this.touched = false;
            for (Node child : this.order) {
                child.reset();
            }
        }
    }

    private Node root = new Node("root", null);
    private final Node[] stack = new Node[512];
    private final long[] starts = new long[512];
    /** Depth of the current section; 0 = root */
    private int depth;
    private boolean inTick;
    private long frames;
    private long tickFrames;
    /** Whether this loop iteration ran a client tick (vanilla counts "clientTick" in each) */
    private boolean ticked;
    private final List<Node> touched = new ArrayList<>();
    private boolean recording;

    private LightProfiler() {
    }

    public static boolean enabled() {
        return MAX_DEPTH > 0;
    }

    public void startRecording() {
        this.root.reset();
        this.frames = 0;
        this.tickFrames = 0;
        this.touched.clear();
        this.recording = true;
    }

    public String stopRecording(String phase) {
        this.recording = false;
        StringBuilder sb = new StringBuilder();
        sb.append(String.format(Locale.ROOT, "Light profile of %s: %d frames, %.3f ms/frame (wall, loop iteration)%n",
                phase, this.frames, perFrame(this.root.nanos)));
        sb.append(String.format(Locale.ROOT, "%-58s %9s %9s %9s %9s%n",
                "section", "ms/frame", "self", "calls/f", "deep/f"));
        for (Node child : this.root.order) {
            append(sb, child, 0);
        }
        long otherFrames = this.frames - this.tickFrames;
        sb.append(String.format(Locale.ROOT, "%nSplit by frame kind: %d frames with a client tick, %d without (ms per frame of each kind)%n",
                this.tickFrames, otherFrames));
        sb.append(String.format(Locale.ROOT, "%-58s %9s %9s %9s%n", "section", "tick f", "other f", "diff"));
        appendSplit(sb, this.root, 0, otherFrames);
        return sb.toString();
    }

    private void appendSplit(StringBuilder sb, Node node, int indent, long otherFrames) {
        double t = node.tickNanos / 1.0e6 / Math.max(1, this.tickFrames);
        double o = node.otherNanos / 1.0e6 / Math.max(1, otherFrames);
        if (node.calls == 0 || Math.max(t, o) < 0.001) {
            return;
        }
        sb.append(String.format(Locale.ROOT, "%-58s %9.4f %9.4f %9.4f%n", "  ".repeat(indent) + node.name, t, o, t - o));
        for (Node child : node.order) {
            appendSplit(sb, child, indent + 1, otherFrames);
        }
    }

    private void append(StringBuilder sb, Node node, int indent) {
        if (node.calls == 0) {
            return;
        }
        long childNanos = 0;
        for (Node child : node.order) {
            childNanos += child.nanos;
        }
        String name = "  ".repeat(indent) + node.name;
        sb.append(String.format(Locale.ROOT, "%-58s %9.4f %9.4f %9.2f %9.1f%n", name, perFrame(node.nanos),
                perFrame(node.nanos - childNanos), (double) node.calls / Math.max(1, this.frames),
                (double) node.deepPushes / Math.max(1, this.frames)));
        node.counters.forEach((counter, amount) -> sb.append(String.format(Locale.ROOT, "%-58s %9.1f per frame%n",
                "  ".repeat(indent + 1) + "#" + counter, (double) amount[0] / Math.max(1, this.frames))));
        for (Node child : node.order) {
            append(sb, child, indent + 1);
        }
    }

    private double perFrame(long nanos) {
        return nanos / 1.0e6 / Math.max(1, this.frames);
    }

    @Override
    public void startTick() {
        this.depth = 0;
        this.stack[0] = this.root;
        this.starts[0] = System.nanoTime();
        this.inTick = true;
    }

    @Override
    public void endTick() {
        // Unbalanced sections (shouldn't happen) are dropped
        if (this.recording && this.inTick) {
            long nanos = System.nanoTime() - this.starts[0];
            this.root.nanos += nanos;
            this.root.calls++;
            this.frames++;
            if (this.ticked) {
                this.tickFrames++;
                this.root.tickNanos += nanos;
            } else {
                this.root.otherNanos += nanos;
            }
            for (Node node : this.touched) {
                if (this.ticked) {
                    node.tickNanos += node.pending;
                } else {
                    node.otherNanos += node.pending;
                }
                node.pending = 0;
                node.touched = false;
            }
        }
        this.touched.clear();
        this.ticked = false;
        this.inTick = false;
        this.depth = 0;
    }

    @Override
    public void push(String name) {
        int d = ++this.depth;
        if (!this.inTick || d >= this.stack.length) {
            return;
        }
        if (d > MAX_DEPTH) {
            this.stack[MAX_DEPTH].deepPushes++;
            return;
        }
        this.stack[d] = this.stack[d - 1].child(name);
        this.starts[d] = System.nanoTime();
    }

    @Override
    public void push(Supplier<String> name) {
        if (this.depth + 1 <= MAX_DEPTH) {
            this.push(name.get());
        } else {
            this.push("");
        }
    }

    @Override
    public void pop() {
        int d = this.depth;
        if (d <= 0) {
            return;
        }
        this.depth = d - 1;
        if (!this.inTick || d > MAX_DEPTH || d >= this.stack.length) {
            return;
        }
        long now = System.nanoTime();
        Node node = this.stack[d];
        if (this.recording) {
            long nanos = now - this.starts[d];
            node.nanos += nanos;
            node.calls++;
            node.pending += nanos;
            if (!node.touched) {
                node.touched = true;
                this.touched.add(node);
            }
        }
    }

    @Override
    public void popPush(String name) {
        this.pop();
        this.push(name);
    }

    @Override
    public void popPush(Supplier<String> name) {
        this.pop();
        this.push(name);
    }

    @Override
    public void markForCharting(MetricCategory category) {
    }

    @Override
    public void incrementCounter(String name, int amount) {
        if (name.equals("clientTick")) {
            this.ticked = true;
        }
        int d = Math.min(this.depth, MAX_DEPTH);
        if (this.recording && this.inTick && d < this.stack.length) {
            this.stack[d].counters.computeIfAbsent(name, _ -> new long[1])[0] += amount;
        }
    }

    @Override
    public void incrementCounter(Supplier<String> name, int amount) {
    }
}
