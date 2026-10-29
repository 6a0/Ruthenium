package net.ruthenium.render.vertex;

import com.mojang.blaze3d.vertex.CompactVectorArray;
import com.mojang.blaze3d.vertex.VertexSorting;
import org.joml.Vector3f;

import java.util.Arrays;

/**
 * Faster drop-in for {@link VertexSorting#byDistance(VertexSorting.DistanceFunction)}, which merge sorts indices
 * with a float comparator. Small inputs are sorted as packed (distance, index) longs, large ones with a stable
 * LSD radix sort on the distance bits. The resulting order is identical to vanilla's (descending distance, ties
 * in ascending index order, matching {@link Float#compare} semantics).
 * <p>
 * Scratch arrays are kept per thread; only the returned index array is allocated per sort.
 * <p>
 * {@link Sorter#sortFrom} sorts starting from a given order of the quads (last frame's order of the same draw,
 * {@link SortOrderCache}) by insertion. Every algorithm here sorts by the same strict total order (distance key, then
 * quad index), whose sorted permutation is unique, so the starting order changes only how long the sort takes, never
 * its result.
 */
public final class DistanceSorting {
    private static final int RADIX_THRESHOLD = 256;

    private static final ThreadLocal<Scratch> SCRATCH = ThreadLocal.withInitial(Scratch::new);

    private DistanceSorting() {
    }

    public static VertexSorting byDistance(VertexSorting.DistanceFunction function) {
        return new Sorter(function);
    }

    /** A sort by distance: vanilla's {@code VertexSorting.byDistance} with the same function */
    public static final class Sorter implements VertexSorting {
        private final DistanceFunction function;

        private Sorter(DistanceFunction function) {
            this.function = function;
        }

        @Override
        public int[] sort(CompactVectorArray values) {
            int size = values.size();
            Scratch scratch = SCRATCH.get();
            computeKeys(scratch, values, size, this.function);
            return size < RADIX_THRESHOLD ? sortPacked(scratch.keys, size) : radixSort(scratch, size);
        }

        /**
         * Sorts the quads into {@code out} (which may be {@code start}) starting from the order {@code start}, a
         * permutation of {@code 0..values.size()-1}. The result is the same as {@link #sort}'s; when the start is far
         * from sorted it falls back to {@link #sort}'s algorithm after a bounded number of moves. Returns {@code true}
         * if the start was close enough to be used.
         */
        public boolean sortFrom(CompactVectorArray values, int[] start, int[] out) {
            int size = values.size();
            Scratch scratch = SCRATCH.get();
            computeKeys(scratch, values, size, this.function);
            int[] keys = scratch.keys;

            long[] packed = scratch.packed;
            for (int j = 0; j < size; j++) {
                int i = start[j];
                packed[j] = ((long) (keys[i] ^ Integer.MIN_VALUE) << 32) | i;
            }

            // Insertion sort; the values are distinct (they hold the quad index)
            int budget = (size << 2) + 64;
            for (int j = 1; j < size; j++) {
                long value = packed[j];
                int k = j - 1;
                if (packed[k] < value) {
                    continue;
                }
                do {
                    packed[k + 1] = packed[k];
                    k--;
                } while (k >= 0 && packed[k] > value);
                packed[k + 1] = value;
                budget -= j - 1 - k;
                if (budget < 0) {
                    int[] sorted = size < RADIX_THRESHOLD ? sortPacked(keys, size) : radixSort(scratch, size);
                    System.arraycopy(sorted, 0, out, 0, size);
                    return false;
                }
            }

            for (int j = 0; j < size; j++) {
                out[j] = (int) packed[j];
            }
            return true;
        }
    }

    /** Keys whose unsigned ascending order is vanilla's descending distance order */
    private static void computeKeys(Scratch scratch, CompactVectorArray values, int size, VertexSorting.DistanceFunction function) {
        scratch.ensureCapacity(size);
        int[] keys = scratch.keys;
        Vector3f point = scratch.point;
        for (int i = 0; i < size; i++) {
            keys[i] = ~sortableBits(function.apply(values.get(i, point))) ^ Integer.MIN_VALUE;
        }
    }

    private static int[] sortPacked(int[] keys, int size) {
        long[] packed = new long[size];
        for (int i = 0; i < size; i++) {
            // Flip the sign bit back so that the signed long order matches the unsigned key order
            packed[i] = ((long) (keys[i] ^ Integer.MIN_VALUE) << 32) | i;
        }

        Arrays.sort(packed);

        int[] indices = new int[size];
        for (int i = 0; i < size; i++) {
            indices[i] = (int) packed[i];
        }
        return indices;
    }

    /** Stable LSD radix sort (8 bits per pass) of the scratch keys, returning the sorted indices */
    private static int[] radixSort(Scratch scratch, int size) {
        int[] result = new int[size];
        for (int i = 0; i < size; i++) {
            result[i] = i;
        }

        int[] keys = scratch.keys;
        int[] indices = result;
        int[] keysTmp = scratch.keysTmp;
        int[] indicesTmp = scratch.indicesTmp;
        int[] counts = scratch.counts;

        for (int shift = 0; shift < 32; shift += 8) {
            Arrays.fill(counts, 0);
            for (int i = 0; i < size; i++) {
                counts[(keys[i] >>> shift) & 0xFF]++;
            }

            // All keys share this byte: the pass wouldn't change the order
            if (counts[(keys[0] >>> shift) & 0xFF] == size) {
                continue;
            }

            int offset = 0;
            for (int b = 0; b < 256; b++) {
                int count = counts[b];
                counts[b] = offset;
                offset += count;
            }

            for (int i = 0; i < size; i++) {
                int key = keys[i];
                int pos = counts[(key >>> shift) & 0xFF]++;
                keysTmp[pos] = key;
                indicesTmp[pos] = indices[i];
            }

            int[] swapKeys = keys;
            keys = keysTmp;
            keysTmp = swapKeys;
            int[] swapIndices = indices;
            indices = indicesTmp;
            indicesTmp = swapIndices;
        }

        if (indices != result) {
            System.arraycopy(indices, 0, result, 0, size);
        }
        return result;
    }

    /** Maps a float to an int whose signed order matches {@link Float#compare} */
    private static int sortableBits(float value) {
        int bits = Float.floatToIntBits(value);
        return bits ^ ((bits >> 31) & 0x7FFFFFFF);
    }

    private static final class Scratch {
        final Vector3f point = new Vector3f();
        final int[] counts = new int[256];
        int[] keys = new int[0];
        int[] keysTmp = new int[0];
        int[] indicesTmp = new int[0];
        long[] packed = new long[0];

        void ensureCapacity(int size) {
            if (this.keys.length < size) {
                int capacity = Math.max(size, this.keys.length * 2);
                this.keys = new int[capacity];
                this.keysTmp = new int[capacity];
                this.indicesTmp = new int[capacity];
                this.packed = new long[capacity];
            }
        }
    }
}
