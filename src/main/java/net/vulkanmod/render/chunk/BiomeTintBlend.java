package net.vulkanmod.render.chunk;

import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.core.BlockPos;
import net.minecraft.util.ARGB;
import net.minecraft.world.level.ColorResolver;

import java.util.Arrays;
import java.util.concurrent.atomic.AtomicInteger;

/**
 * Vanilla's {@code ClientLevel.calculateBlockTint} with the per-column samples cached: a block's tint is the average
 * of {@code resolver.getColor(biome, x, z)} over the (2r+1)^2 columns around it at its height, and each sample needs a
 * fuzzy-zoomed biome lookup. Neighbouring blocks share almost all of their samples, so each thread keeps the samples
 * of the section it last asked about (per resolver) and computes each one once. The sum and the division are the
 * same integer arithmetic as vanilla's, so the result is identical.
 * <p>
 * The samples are dropped whenever a chunk's biomes may have changed (chunk load or unload, vanilla clearing its tint
 * caches) through {@link #invalidate()}.
 */
public final class BiomeTintBlend {
    private static final int MAX_RADIUS = 16;
    private static final int RESOLVER_SLOTS = 8;

    private static final AtomicInteger GENERATION = new AtomicInteger();
    private static final ThreadLocal<Samples[]> SAMPLES = ThreadLocal.withInitial(() -> new Samples[RESOLVER_SLOTS]);

    private BiomeTintBlend() {
    }

    public static void invalidate() {
        GENERATION.incrementAndGet();
    }

    /** Returns vanilla's blended tint at {@code pos}, for a blend radius in [1, {@value MAX_RADIUS}] */
    public static int blend(ClientLevel level, BlockPos pos, ColorResolver resolver, int radius) {
        Samples samples = samples(resolver);
        int x = pos.getX();
        int y = pos.getY();
        int z = pos.getZ();
        samples.prepare(level, x >> 4, y >> 4, z >> 4, radius);

        int size = samples.size;
        int rowStart = ((y & 15) * size + (z & 15)) * size + (x & 15);
        int totalRed = 0;
        int totalGreen = 0;
        int totalBlue = 0;
        for (int dz = 0, diameter = radius * 2 + 1; dz < diameter; dz++) {
            int index = rowStart + dz * size;
            for (int dx = 0; dx < diameter; dx++, index++) {
                int color = samples.get(index, x - radius + dx, y, z - radius + dz);
                totalRed += ARGB.red(color);
                totalGreen += ARGB.green(color);
                totalBlue += ARGB.blue(color);
            }
        }
        int count = (radius * 2 + 1) * (radius * 2 + 1);
        return ARGB.color(totalRed / count, totalGreen / count, totalBlue / count);
    }

    private static Samples samples(ColorResolver resolver) {
        Samples[] slots = SAMPLES.get();
        for (int i = 0; i < RESOLVER_SLOTS; i++) {
            Samples samples = slots[i];
            if (samples == null) {
                return slots[i] = new Samples(resolver);
            }
            if (samples.resolver == resolver) {
                return samples;
            }
        }
        // More resolvers than slots (mods can register their own): reuse the last slot
        Samples samples = new Samples(resolver);
        slots[RESOLVER_SLOTS - 1] = samples;
        return samples;
    }

    /** One resolver's samples for a section and the columns within the blend radius around it, 16 heights */
    private static final class Samples {
        final ColorResolver resolver;
        final BlockPos.MutableBlockPos samplePos = new BlockPos.MutableBlockPos();
        ClientLevel level;
        int sectionX, sectionY, sectionZ, radius = -1, generation;
        int size;
        int[] colors = new int[0];
        /** An entry is valid when its stamp is {@link #stamp}, so starting over needs no clearing */
        int[] stamps = new int[0];
        int stamp;

        Samples(ColorResolver resolver) {
            this.resolver = resolver;
        }

        void prepare(ClientLevel level, int sectionX, int sectionY, int sectionZ, int radius) {
            int generation = GENERATION.get();
            if (this.level == level && this.sectionX == sectionX && this.sectionY == sectionY && this.sectionZ == sectionZ
                    && this.radius == radius && this.generation == generation) {
                return;
            }
            this.level = level;
            this.sectionX = sectionX;
            this.sectionY = sectionY;
            this.sectionZ = sectionZ;
            this.generation = generation;
            if (this.radius != radius) {
                this.radius = radius;
                this.size = 16 + radius * 2;
                int entries = 16 * this.size * this.size;
                if (this.colors.length < entries) {
                    this.colors = new int[entries];
                    this.stamps = new int[entries];
                    this.stamp = 0;
                }
            }
            if (++this.stamp == 0) {
                Arrays.fill(this.stamps, 0);
                this.stamp = 1;
            }
        }

        int get(int index, int x, int y, int z) {
            if (this.stamps[index] == this.stamp) {
                return this.colors[index];
            }
            int color = this.resolver.getColor(this.level.getBiome(this.samplePos.set(x, y, z)).value(), x, z);
            this.colors[index] = color;
            this.stamps[index] = this.stamp;
            return color;
        }
    }

    public static boolean supports(int radius) {
        return radius >= 1 && radius <= MAX_RADIUS;
    }
}
