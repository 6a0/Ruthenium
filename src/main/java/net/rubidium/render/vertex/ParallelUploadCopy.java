package net.rubidium.render.vertex;

import org.lwjgl.system.MemoryUtil;

import java.util.Arrays;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.atomic.AtomicInteger;

/**
 * Copies the vertex data of a {@code StagedVertexBuffer} upload into the mapped staging buffer on several threads.
 * <p>
 * Vanilla copies every slice with one {@code ByteBuffer.put} on the render thread. With many block entities in view
 * that is ~34 MB per frame into write-combined memory, ~3.3 ms at ~10 GB/s. Here the copies are only recorded
 * ({@link #add}); {@link #start} hands them to worker threads in {@link #CHUNK_SIZE} chunks while the render thread
 * goes on to sort the translucent draws, and {@link #finish} lets the render thread copy the remaining chunks and
 * waits for all of them, before the staging buffer is unmapped. Small uploads are copied directly.
 * <p>
 * Only used from the render thread, one upload at a time.
 */
public final class ParallelUploadCopy {
    private static final int CHUNK_SIZE = 1 << 20;
    private static final long MIN_PARALLEL_SIZE = 4L * CHUNK_SIZE;
    private static final int WORKERS = Runtime.getRuntime().availableProcessors() >= 4 ? 1 : 0;
    private static final ExecutorService EXECUTOR = WORKERS > 0 ? Executors.newFixedThreadPool(WORKERS, runnable -> {
        Thread thread = new Thread(runnable, "Rubidium upload copy");
        thread.setDaemon(true);
        return thread;
    }) : null;

    private static long[] srcs = new long[64];
    private static long[] dsts = new long[64];
    /** Offset of each segment in the stream of all copied bytes; one extra entry holds the total */
    private static long[] offsets = new long[65];
    private static int segmentCount;
    private static long totalSize;

    private static Job job;

    private ParallelUploadCopy() {
    }

    public static void add(long src, long dst, long size) {
        if (size <= 0) {
            return;
        }

        int i = segmentCount;
        if (i == srcs.length) {
            srcs = Arrays.copyOf(srcs, i * 2);
            dsts = Arrays.copyOf(dsts, i * 2);
            offsets = Arrays.copyOf(offsets, i * 2 + 1);
        }
        srcs[i] = src;
        dsts[i] = dst;
        offsets[i] = totalSize;
        totalSize += size;
        offsets[i + 1] = totalSize;
        segmentCount = i + 1;
    }

    /** Lets the workers begin copying what was added so far; nothing may be added afterwards */
    public static void start() {
        if (job != null || segmentCount == 0) {
            return;
        }

        Job job = new Job((int) ((totalSize + CHUNK_SIZE - 1) / CHUNK_SIZE), totalSize);
        ParallelUploadCopy.job = job;
        if (EXECUTOR != null && totalSize >= MIN_PARALLEL_SIZE) {
            for (int i = 0, workers = Math.min(WORKERS, job.chunkCount - 1); i < workers; i++) {
                EXECUTOR.execute(job);
            }
        }
    }

    /** Copies whatever the workers haven't taken and waits until everything has been copied */
    public static void finish() {
        if (segmentCount == 0) {
            return;
        }

        start();
        Job job = ParallelUploadCopy.job;
        job.run();
        while (job.done.get() < job.chunkCount) {
            Thread.onSpinWait();
        }

        ParallelUploadCopy.job = null;
        segmentCount = 0;
        totalSize = 0;
    }

    /**
     * One upload's chunks. A worker that only gets to run after the upload finished finds no chunk left and
     * returns without touching the segment arrays, which the next upload may already be filling.
     */
    private static final class Job implements Runnable {
        final int chunkCount;
        final long totalSize;
        final AtomicInteger next = new AtomicInteger();
        final AtomicInteger done = new AtomicInteger();

        Job(int chunkCount, long totalSize) {
            this.chunkCount = chunkCount;
            this.totalSize = totalSize;
        }

        @Override
        public void run() {
            int chunk;
            while ((chunk = this.next.getAndIncrement()) < this.chunkCount) {
                copyRange((long) chunk * CHUNK_SIZE, Math.min(this.totalSize, (long) (chunk + 1) * CHUNK_SIZE));
                this.done.incrementAndGet();
            }
        }
    }

    private static void copyRange(long from, long to) {
        long[] offsets = ParallelUploadCopy.offsets;
        int segment = Arrays.binarySearch(offsets, 0, segmentCount, from);
        if (segment < 0) {
            segment = -segment - 2;
        }

        // Skip empty segments that share the start offset
        while (offsets[segment + 1] <= from) {
            segment++;
        }

        long position = from;
        while (position < to) {
            long segmentEnd = Math.min(offsets[segment + 1], to);
            long skip = position - offsets[segment];
            MemoryUtil.memCopy(srcs[segment] + skip, dsts[segment] + skip, segmentEnd - position);
            position = segmentEnd;
            segment++;
        }
    }
}
