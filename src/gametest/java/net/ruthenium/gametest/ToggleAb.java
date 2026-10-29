package net.ruthenium.gametest;

import net.fabricmc.fabric.api.client.gametest.v1.context.ClientGameTestContext;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;

/**
 * An A/B of one boolean {@code Config} switch inside one launch ({@code bench.toggleField=<field>}): after a scene's
 * phases, {@code bench.toggleRounds} rounds (default 8) of the static view and of the pan, each round measuring both
 * settings in ABBA order. Each side discards {@code bench.toggleWarmup} seconds (default 1) after the flip, then measures
 * {@code bench.toggleSeconds} (default 3) of the static view or one full pan of {@code bench.togglePanSeconds} (default 5).
 * Logs each round and a summary: per side median fps and render thread CPU ms per frame, and the median and mean of the
 * per-round ratio. Both settings run on code the JIT compiled for both, so a difference here is the switch's direct
 * cost; a cross-launch difference that doesn't show here points at compilation or layout.
 */
final class ToggleAb {
    private static final Logger LOGGER = LoggerFactory.getLogger("Ruthenium Toggle A/B");
    static final String FIELD = BenchmarkClientGameTest.prop("toggleField", "");
    private static final int ROUNDS = Integer.parseInt(BenchmarkClientGameTest.prop("toggleRounds", "8"));
    private static final double SECONDS = Double.parseDouble(BenchmarkClientGameTest.prop("toggleSeconds", "3"));
    private static final double WARMUP = Double.parseDouble(BenchmarkClientGameTest.prop("toggleWarmup", "1"));
    private static final double PAN_SECONDS = Double.parseDouble(BenchmarkClientGameTest.prop("togglePanSeconds", "5"));
    /** {@code bench.togglePhases}: static, pan or both (default) */
    private static final String PHASES = BenchmarkClientGameTest.prop("togglePhases", "static,pan");

    private ToggleAb() {
    }

    static boolean enabled() {
        return !FIELD.isEmpty();
    }

    /**
     * Runs the rounds: {@code resetView} puts the camera back at the static view, {@code startYaw}/{@code pitch} start
     * the pans
     */
    static void run(ClientGameTestContext context, List<PhaseResult> results, Runnable resetView, float startYaw, float pitch) {
        boolean initial = get(context);
        if (PHASES.contains("static")) {
            resetView.run();
            rounds(context, results, "static", initial, () -> BenchmarkClientGameTest.waitSeconds(context, SECONDS), resetView);
        }
        if (PHASES.contains("pan")) {
            resetView.run();
            rounds(context, results, "pan", initial, () -> {
                context.runOnClient(mc -> FrameRecorder.startPan(mc, startYaw, pitch, PAN_SECONDS));
                context.waitFor(_ -> !FrameRecorder.isPanning(), (int) (PAN_SECONDS * 20 * 10));
            }, resetView);
        }
        set(context, initial);
    }

    private static void rounds(ClientGameTestContext context, List<PhaseResult> results, String phase, boolean initial,
                               Runnable measure, Runnable resetView) {
        List<double[]> a = new ArrayList<>();
        List<double[]> b = new ArrayList<>();
        for (int round = 0; round < ROUNDS; round++) {
            boolean[] order = round % 2 == 0 ? new boolean[]{initial, !initial} : new boolean[]{!initial, initial};
            for (boolean value : order) {
                set(context, value);
                BenchmarkClientGameTest.waitSeconds(context, WARMUP);
                context.runOnClient(_ -> FrameRecorder.startPhase());
                measure.run();
                String name = "%s-%s-%d".formatted(phase, value ? "on" : "off", round);
                PhaseResult result = context.computeOnClient(_ -> FrameRecorder.stopPhase(name, -1));
                results.add(result);
                LOGGER.info(result.summaryLine());
                (value == initial ? a : b).add(new double[]{result.avgFps(), result.renderThreadCpuMsPerFrame(), result.p99LowFps()});
                if (phase.equals("pan")) {
                    resetView.run();
                }
            }
        }
        double[] ratios = new double[ROUNDS];
        for (int i = 0; i < ROUNDS; i++) {
            ratios[i] = b.get(i)[0] / a.get(i)[0];
        }
        LOGGER.info("Toggle A/B {} {}: {}={} fps {} (median {}, cpu {} ms, p99 {}), {}={} fps {} (median {}, cpu {} ms, p99 {}); "
                        + "{}/{} per round {} (median {}, mean {}, {} of {} higher)",
                phase, FIELD,
                FIELD, initial, fmt(a, 0), f(median(a, 0)), f3(median(a, 1)), f(median(a, 2)),
                FIELD, !initial, fmt(b, 0), f(median(b, 0)), f3(median(b, 1)), f(median(b, 2)),
                !initial ? "on" : "off", initial ? "on" : "off",
                Arrays.toString(Arrays.stream(ratios).mapToObj(r -> String.format("%.4f", r)).toArray()),
                String.format("%.4f", median(ratios)), String.format("%.4f", Arrays.stream(ratios).average().orElse(0)),
                Arrays.stream(ratios).filter(r -> r > 1).count(), ROUNDS);
    }

    private static String fmt(List<double[]> rows, int column) {
        StringBuilder sb = new StringBuilder("[");
        for (double[] row : rows) {
            sb.append(sb.length() > 1 ? " " : "").append(f(row[column]));
        }
        return sb.append(']').toString();
    }

    private static String f(double v) {
        return String.format("%.1f", v);
    }

    private static String f3(double v) {
        return String.format("%.3f", v);
    }

    private static double median(List<double[]> rows, int column) {
        return median(rows.stream().mapToDouble(r -> r[column]).toArray());
    }

    private static double median(double[] values) {
        double[] sorted = values.clone();
        Arrays.sort(sorted);
        int n = sorted.length;
        return n == 0 ? 0 : n % 2 == 1 ? sorted[n / 2] : (sorted[n / 2 - 1] + sorted[n / 2]) / 2;
    }

    private static boolean get(ClientGameTestContext context) {
        return context.computeOnClient(_ -> {
            try {
                Object config = Class.forName("net.ruthenium.Initializer").getField("CONFIG").get(null);
                return config.getClass().getField(FIELD).getBoolean(config);
            } catch (ReflectiveOperationException e) {
                throw new AssertionError("Can't read " + FIELD, e);
            }
        });
    }

    private static void set(ClientGameTestContext context, boolean value) {
        context.runOnClient(_ -> {
            try {
                Object config = Class.forName("net.ruthenium.Initializer").getField("CONFIG").get(null);
                config.getClass().getField(FIELD).setBoolean(config, value);
            } catch (ReflectiveOperationException e) {
                throw new AssertionError("Can't toggle " + FIELD, e);
            }
        });
    }
}
