package net.ruthenium.render.vertex;

import com.mojang.blaze3d.vertex.CompactVectorArray;
import it.unimi.dsi.fastutil.longs.Long2ObjectMap;
import it.unimi.dsi.fastutil.longs.Long2ObjectOpenHashMap;
import it.unimi.dsi.fastutil.objects.ObjectIterator;

/**
 * Sorted draws of a {@code StagedVertexBuffer} start their quad sort from the order the same draw had in its last
 * upload ({@code reuseSortOrder}). A still model under a still camera (or a turning one: the view rotation is not in
 * the vertex positions) has the same or nearly the same order every frame, so the insertion sort of
 * {@link DistanceSorting.Sorter#sortFrom} only checks it.
 * <p>
 * Exactness doesn't depend on finding the right draw: the sort's result is the unique ordering of this frame's keys
 * whatever order it starts from (see {@link DistanceSorting}), so a draw is recognised by a cheap guess, its quad count
 * and the block half its first quad's centroid lies in (camera-relative). A wrong guess (another player with as many
 * quads) only costs time, and a start that is far from sorted is abandoned after a bounded number of moves. A draw
 * without an entry is sorted from scratch.
 * <p>
 * Entries not used for {@link #KEEP_UPLOADS} uploads are dropped. Render thread only (one cache per buffer).
 */
public final class SortOrderCache {
    /** Draws with fewer quads are sorted directly */
    private static final int MIN_QUADS = 2;
    /**
     * Uploads an entry is kept for without being used. One buffer may be uploaded more than once per frame (the level's
     * features, then the hand's), so this is a few frames' worth.
     */
    private static final int KEEP_UPLOADS = 4;

    private final Long2ObjectOpenHashMap<Entry> entries = new Long2ObjectOpenHashMap<>();
    private int generation;

    /** Called at the start of each upload */
    public void beginUpload() {
        int keep = this.generation - KEEP_UPLOADS;
        this.generation++;
        if (!this.entries.isEmpty()) {
            for (ObjectIterator<Long2ObjectMap.Entry<Entry>> it = this.entries.long2ObjectEntrySet().fastIterator(); it.hasNext(); ) {
                if (it.next().getValue().lastUsed < keep) {
                    it.remove();
                }
            }
        }
    }

    /** The sorted quad order of the draw with these centroids, as {@code sorter.sort(centroids)} would return it */
    public int[] sort(DistanceSorting.Sorter sorter, CompactVectorArray centroids) {
        int size = centroids.size();
        if (size < MIN_QUADS) {
            return sorter.sort(centroids);
        }

        long key = key(centroids, size);
        Entry entry = this.entries.get(key);
        if (entry != null && entry.order.length == size) {
            sorter.sortFrom(centroids, entry.order, entry.order);
        } else {
            entry = new Entry(sorter.sort(centroids));
            this.entries.put(key, entry);
        }

        entry.lastUsed = this.generation;
        return entry.order;
    }

    public void clear() {
        this.entries.clear();
    }

    /** Quad count and the block half of the first quad's centroid */
    private static long key(CompactVectorArray centroids, int size) {
        long x = (long) Math.floor(centroids.getX(0) * 2.0F) & 0xFFFFF;
        long y = (long) Math.floor(centroids.getY(0) * 2.0F) & 0xFFFFF;
        long z = (long) Math.floor(centroids.getZ(0) * 2.0F) & 0xFFFFF;
        return (size * 0x9E3779B97F4A7C15L) ^ (x << 40 | y << 20 | z);
    }

    private static final class Entry {
        final int[] order;
        int lastUsed;

        Entry(int[] order) {
            this.order = order;
        }
    }
}
