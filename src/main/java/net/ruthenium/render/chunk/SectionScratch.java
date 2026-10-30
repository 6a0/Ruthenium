package net.ruthenium.render.chunk;

import net.minecraft.world.level.block.state.BlockState;
import net.ruthenium.Initializer;

import java.util.Arrays;

/**
 * The scratch arrays of one section compile: the block states around the section ({@code RenderSectionRegionM}), its
 * smooth lighting window ({@code BlockModelLighterCacheM}) and its biome tint grid ({@link BiomeTintBlend}), each
 * allocated on first use.
 * <p>
 * They used to be per thread, so every background worker that ever compiled a section kept ~0.5 MB of them for good
 * (about 20 threads here, more on bigger CPUs). With switch {@code freeUnusedChunkData} a compile borrows a scratch
 * from a shared pool and returns it when it is done, and pooled scratch unused for {@link #IDLE_NANOS} is dropped, so
 * what stays allocated follows the compiles that actually run at once. A scratch is used by one compile (one thread)
 * at a time; the pool's lock hands it over between threads. With the switch off each thread keeps its own, as before.
 * <p>
 * Entries are valid while their (byte) stamp is the current one, so a new section needs no clearing; when a stamp
 * wraps after 255 sections its array is cleared once.
 */
public final class SectionScratch {
    /** Pooled scratch unused for this long is dropped (checked every frame, see {@link #trim()}) */
    private static final long IDLE_NANOS = 2_000_000_000L;
    public static final int WINDOW = 18;
    private static final int WINDOW_ENTRIES = WINDOW * WINDOW * WINDOW;

    private static final Object LOCK = new Object();
    /** Free scratch, the least recently returned first */
    private static SectionScratch[] pool = new SectionScratch[16];
    private static int pooled;
    private static final ThreadLocal<SectionScratch> PER_THREAD = ThreadLocal.withInitial(() -> new SectionScratch(false));

    private final boolean fromPool;
    private long returnedAt;
    /** Counts the regions this scratch was bound to, so a region can tell whether it still is */
    private int binds;
    private States states;
    private Light light;
    BiomeTintBlend.Grid tintGrid;

    private SectionScratch(boolean fromPool) {
        this.fromPool = fromPool;
    }

    /** A scratch for one compile (or one tint lookup), to be given back with {@link #release()} on the same thread */
    public static SectionScratch borrow() {
        if (!Initializer.CONFIG.freeUnusedChunkData) {
            return PER_THREAD.get();
        }
        synchronized (LOCK) {
            if (pooled > 0) {
                SectionScratch scratch = pool[--pooled];
                pool[pooled] = null;
                return scratch;
            }
        }
        return new SectionScratch(true);
    }

    public void release() {
        if (!this.fromPool) {
            return;
        }
        long now = System.nanoTime();
        synchronized (LOCK) {
            this.returnedAt = now;
            if (pooled == pool.length) {
                pool = Arrays.copyOf(pool, pooled * 2);
            }
            pool[pooled++] = this;
        }
    }

    /** Drops the pooled scratch that has been unused for a while; called every frame on the render thread */
    public static void trim() {
        long now = System.nanoTime();
        synchronized (LOCK) {
            int idle = 0;
            while (idle < pooled && now - pool[idle].returnedAt > IDLE_NANOS) {
                idle++;
            }
            if (idle > 0) {
                System.arraycopy(pool, idle, pool, 0, pooled - idle);
                Arrays.fill(pool, pooled - idle, pooled, null);
                pooled -= idle;
            }
        }
    }

    /** Drops all pooled scratch (leaving a world) */
    public static void clear() {
        synchronized (LOCK) {
            Arrays.fill(pool, 0, pooled, null);
            pooled = 0;
        }
    }

    /** Starts a new binding to a region (see {@code RenderSectionRegionM}) and returns its number */
    public int bind() {
        return ++this.binds;
    }

    public int binds() {
        return this.binds;
    }

    /** The block state window, with a new stamp, or null while switch {@code sectionStateCache} is off */
    public States newStates() {
        if (!Initializer.CONFIG.sectionStateCache) {
            return null;
        }
        States states = this.states;
        if (states == null) {
            states = this.states = new States();
        }
        states.next();
        return states;
    }

    /** The light window, with a new stamp */
    public Light newLight() {
        Light light = this.light;
        if (light == null) {
            light = this.light = new Light();
        }
        light.next();
        return light;
    }

    BiomeTintBlend.Grid tintGrid() {
        BiomeTintBlend.Grid grid = this.tintGrid;
        if (grid == null) {
            grid = this.tintGrid = new BiomeTintBlend.Grid();
        }
        return grid;
    }

    /** Block states of the section and the blocks around it ({@value WINDOW}^3) */
    public static final class States {
        public final BlockState[] states = new BlockState[WINDOW_ENTRIES];
        public final byte[] stamps = new byte[WINDOW_ENTRIES];
        public byte stamp;

        void next() {
            if (++this.stamp == 0) {
                Arrays.fill(this.stamps, (byte) 0);
                this.stamp = 1;
            }
        }
    }

    /** Smooth lighting's light and shade values of the section and the blocks around it ({@value WINDOW}^3) */
    public static final class Light {
        public final int[] light = new int[WINDOW_ENTRIES];
        public final byte[] lightStamps = new byte[WINDOW_ENTRIES];
        public final float[] shade = new float[WINDOW_ENTRIES];
        public final byte[] shadeStamps = new byte[WINDOW_ENTRIES];
        public byte stamp;

        void next() {
            if (++this.stamp == 0) {
                Arrays.fill(this.lightStamps, (byte) 0);
                Arrays.fill(this.shadeStamps, (byte) 0);
                this.stamp = 1;
            }
        }
    }
}
