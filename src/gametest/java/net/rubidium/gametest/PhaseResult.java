package net.rubidium.gametest;

import java.util.Arrays;

/**
 * Frame time and allocation statistics for one benchmark phase.
 * "pXX low" is the FPS at the XXth percentile frame time; "worst X% avg" is the average FPS over the slowest X% of frames.
 */
public record PhaseResult(
        String name,
        int frames,
        double seconds,
        double avgFps,
        double p95LowFps,
        double p99LowFps,
        double worst5PctAvgFps,
        double worst1PctAvgFps,
        double medianFrameMs,
        double maxFrameMs,
        double allocMBps,
        double renderThreadAllocMBps,
        long gcCount,
        long gcMillis,
        int peakParticles,
        double avgEntitiesRendered,
        double avgBlockEntitiesRendered,
        double renderThreadCpuMsPerFrame
) {
    private static final double MB = 1024.0 * 1024.0;

    static PhaseResult compute(String name, long[] frameNanos, long elapsedNanos, long totalAllocBytes,
                               long renderThreadAllocBytes, long gcCount, long gcMillis, int peakParticles,
                               double avgEntities, double avgBlockEntities, long renderThreadCpuNanos) {
        double seconds = elapsedNanos / 1.0e9;
        int n = frameNanos.length;

        if (n == 0) {
            return new PhaseResult(name, 0, seconds, 0, 0, 0, 0, 0, 0, 0,
                    totalAllocBytes / MB / seconds, renderThreadAllocBytes / MB / seconds, gcCount, gcMillis, peakParticles,
                    avgEntities, avgBlockEntities, 0);
        }

        long[] sorted = frameNanos.clone();
        Arrays.sort(sorted);

        long sum = 0;
        for (long t : sorted) {
            sum += t;
        }

        return new PhaseResult(
                name,
                n,
                seconds,
                n / (sum / 1.0e9),
                fps(percentile(sorted, 0.95)),
                fps(percentile(sorted, 0.99)),
                worstAvgFps(sorted, 0.05),
                worstAvgFps(sorted, 0.01),
                percentile(sorted, 0.50) / 1.0e6,
                sorted[n - 1] / 1.0e6,
                totalAllocBytes / MB / seconds,
                renderThreadAllocBytes / MB / seconds,
                gcCount,
                gcMillis,
                peakParticles,
                avgEntities,
                avgBlockEntities,
                renderThreadCpuNanos / 1.0e6 / n
        );
    }

    private static long percentile(long[] sorted, double p) {
        int idx = (int) Math.ceil(p * sorted.length) - 1;
        return sorted[Math.clamp(idx, 0, sorted.length - 1)];
    }

    private static double worstAvgFps(long[] sorted, double fraction) {
        int count = Math.max(1, (int) Math.round(sorted.length * fraction));
        long sum = 0;
        for (int i = sorted.length - count; i < sorted.length; i++) {
            sum += sorted[i];
        }
        return count / (sum / 1.0e9);
    }

    private static double fps(long frameNanos) {
        return 1.0e9 / frameNanos;
    }

    public String summaryLine() {
        return String.format(
                "%-10s frames=%6d  avg=%7.1f fps  p95 low=%7.1f  p99 low=%7.1f  (worst 5%%=%6.1f, worst 1%%=%6.1f)  " +
                "median=%6.2f ms  max=%7.2f ms  alloc=%7.1f MB/s (render thread %7.1f MB/s)  GCs=%d (%d ms)  " +
                "entities=%.0f block entities=%.0f  render thread CPU=%.2f ms/frame%s",
                name, frames, avgFps, p95LowFps, p99LowFps, worst5PctAvgFps, worst1PctAvgFps,
                medianFrameMs, maxFrameMs, allocMBps, renderThreadAllocMBps, gcCount, gcMillis,
                avgEntitiesRendered, avgBlockEntitiesRendered, renderThreadCpuMsPerFrame,
                peakParticles >= 0 ? "  peak particles=" + peakParticles : "");
    }
}
