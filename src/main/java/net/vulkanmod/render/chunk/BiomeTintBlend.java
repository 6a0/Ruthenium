package net.vulkanmod.render.chunk;

import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.core.BlockPos;
import net.minecraft.util.ARGB;
import net.minecraft.util.LinearCongruentialGenerator;
import net.minecraft.util.Mth;
import net.minecraft.world.level.ColorResolver;
import net.minecraft.world.level.biome.Biome;
import net.minecraft.world.level.biome.BiomeManager;
import net.vulkanmod.mixin.render.chunk.BiomeManagerAccessor;

import java.util.Arrays;
import java.util.concurrent.atomic.AtomicInteger;

/**
 * Vanilla's {@code ClientLevel.calculateBlockTint} with the per-column samples cached: a block's tint is the average
 * of {@code resolver.getColor(biome, x, z)} over the (2r+1)^2 columns around it at its height, and each sample needs a
 * fuzzy-zoomed biome lookup. Neighbouring blocks share almost all of their samples, so each thread keeps the biomes
 * of the section it last asked about (shared by all resolvers), each resolver's sample colours and its blended
 * results, and computes each of them once. The sum and the division are the same integer arithmetic as vanilla's,
 * so the result is identical.
 * <p>
 * The biomes themselves are {@code BiomeManager.getBiome}'s fuzzy zoom with its per-corner work cached: the 8 quart
 * corners around a block each get a pseudo-random offset (8 LCG steps) and a noise biome, and the corners are shared
 * by many blocks. The distances are compared with the same double arithmetic, so the chosen corner is the same.
 * <p>
 * Everything is dropped whenever a chunk's biomes may have changed (chunk load or unload, vanilla clearing its tint
 * caches) through {@link #invalidate()}.
 */
public final class BiomeTintBlend {
    private static final int MAX_RADIUS = 16;
    private static final int RESOLVER_SLOTS = 8;

    private static final AtomicInteger GENERATION = new AtomicInteger();
    private static final ThreadLocal<Grid> GRID = ThreadLocal.withInitial(Grid::new);

    private BiomeTintBlend() {
    }

    public static void invalidate() {
        GENERATION.incrementAndGet();
    }

    public static boolean supports(int radius) {
        return radius >= 1 && radius <= MAX_RADIUS;
    }

    /** Returns vanilla's blended tint at {@code pos}, for a blend radius in [1, {@value MAX_RADIUS}] */
    public static int blend(ClientLevel level, BlockPos pos, ColorResolver resolver, int radius) {
        int x = pos.getX();
        int y = pos.getY();
        int z = pos.getZ();
        Grid grid = GRID.get();
        grid.prepare(level, x >> 4, y >> 4, z >> 4, radius);
        Colors colors = grid.colors(resolver);

        int block = ((y & 15) << 8) | ((z & 15) << 4) | (x & 15);
        if (colors.blendStamps[block] == grid.stamp) {
            return colors.blends[block];
        }

        int size = grid.size;
        int rowStart = ((y & 15) * size + (z & 15)) * size + (x & 15);
        int totalRed = 0;
        int totalGreen = 0;
        int totalBlue = 0;
        for (int dz = 0, diameter = radius * 2 + 1; dz < diameter; dz++) {
            int index = rowStart + dz * size;
            for (int dx = 0; dx < diameter; dx++, index++) {
                int color;
                if (colors.sampleStamps[index] == grid.stamp) {
                    color = colors.samples[index];
                } else {
                    int sx = x - radius + dx;
                    int sz = z - radius + dz;
                    color = resolver.getColor(grid.biome(index, sx, y, sz), sx, sz);
                    colors.samples[index] = color;
                    colors.sampleStamps[index] = grid.stamp;
                }
                totalRed += ARGB.red(color);
                totalGreen += ARGB.green(color);
                totalBlue += ARGB.blue(color);
            }
        }
        int count = (radius * 2 + 1) * (radius * 2 + 1);
        int blended = ARGB.color(totalRed / count, totalGreen / count, totalBlue / count);
        colors.blends[block] = blended;
        colors.blendStamps[block] = grid.stamp;
        return blended;
    }

    /**
     * A section and the columns within the blend radius around it, 16 heights. An entry is valid when its stamp is
     * the grid's, so starting over needs no clearing.
     */
    private static final class Grid {
        final BlockPos.MutableBlockPos samplePos = new BlockPos.MutableBlockPos();
        final Colors[] slots = new Colors[RESOLVER_SLOTS];
        ClientLevel level;
        int sectionX, sectionY, sectionZ, radius = -1, generation;
        int size;
        int entries;
        Biome[] biomes = new Biome[0];
        int[] biomeStamps = new int[0];
        int stamp;

        /** The level's biome manager, if it is vanilla's (its fuzzy zoom is reproduced here), else null */
        BiomeManager biomeManager;
        long zoomSeed;
        /** Quart corners around the grid's blocks: x, y, z offsets per corner and its noise biome */
        int cornerMinX, cornerMinY, cornerMinZ, cornersX, cornersY, cornersZ;
        double[] fiddles = new double[0];
        int[] fiddleStamps = new int[0];
        Biome[] cornerBiomes = new Biome[0];
        int[] cornerBiomeStamps = new int[0];

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
            BiomeManager manager = level.getBiomeManager();
            this.biomeManager = manager.getClass() == BiomeManager.class ? manager : null;
            if (this.biomeManager != null) {
                this.zoomSeed = ((BiomeManagerAccessor) manager).vulkanmod$getBiomeZoomSeed();
            }
            if (this.radius != radius) {
                this.radius = radius;
                this.size = 16 + radius * 2;
                this.entries = 16 * this.size * this.size;
                // Corners of the blocks' parent cells (fuzzy zoom: block - 2) and the cells one further
                this.cornersX = this.cornersZ = ((16 + radius * 2 + 2) >> 2) + 3;
                this.cornersY = ((16 + 2) >> 2) + 3;
                int corners = this.cornersX * this.cornersY * this.cornersZ;
                if (this.cornerBiomes.length < corners) {
                    this.fiddles = new double[corners * 3];
                    this.fiddleStamps = new int[corners];
                    this.cornerBiomes = new Biome[corners];
                    this.cornerBiomeStamps = new int[corners];
                }
                if (this.biomes.length < this.entries) {
                    this.biomes = new Biome[this.entries];
                    this.biomeStamps = new int[this.entries];
                    for (Colors colors : this.slots) {
                        if (colors != null) {
                            colors.resize(this.entries);
                        }
                    }
                }
            }
            this.cornerMinX = ((sectionX << 4) - radius - 2) >> 2;
            this.cornerMinY = ((sectionY << 4) - 2) >> 2;
            this.cornerMinZ = ((sectionZ << 4) - radius - 2) >> 2;
            if (++this.stamp == 0) {
                Arrays.fill(this.biomeStamps, 0);
                Arrays.fill(this.fiddleStamps, 0);
                Arrays.fill(this.cornerBiomeStamps, 0);
                for (Colors colors : this.slots) {
                    if (colors != null) {
                        colors.clearStamps();
                    }
                }
                this.stamp = 1;
            }
        }

        Biome biome(int index, int x, int y, int z) {
            if (this.biomeStamps[index] == this.stamp) {
                return this.biomes[index];
            }
            Biome biome = this.biomeManager != null ? this.zoomedBiome(x, y, z)
                    : this.level.getBiome(this.samplePos.set(x, y, z)).value();
            this.biomes[index] = biome;
            this.biomeStamps[index] = this.stamp;
            return biome;
        }

        /** {@code BiomeManager.getBiome(x, y, z)} with the corners' offsets and noise biomes cached */
        private Biome zoomedBiome(int x, int y, int z) {
            int absX = x - 2;
            int absY = y - 2;
            int absZ = z - 2;
            int parentX = absX >> 2;
            int parentY = absY >> 2;
            int parentZ = absZ >> 2;
            double fractX = (absX & 3) / 4.0;
            double fractY = (absY & 3) / 4.0;
            double fractZ = (absZ & 3) / 4.0;
            int minI = 0;
            double minFiddledDistance = Double.POSITIVE_INFINITY;

            for (int i = 0; i < 8; i++) {
                boolean xEven = (i & 4) == 0;
                boolean yEven = (i & 2) == 0;
                boolean zEven = (i & 1) == 0;
                int corner = this.fiddle(xEven ? parentX : parentX + 1, yEven ? parentY : parentY + 1, zEven ? parentZ : parentZ + 1);
                double distanceX = xEven ? fractX : fractX - 1.0;
                double distanceY = yEven ? fractY : fractY - 1.0;
                double distanceZ = zEven ? fractZ : fractZ - 1.0;
                double next = Mth.square(distanceZ + this.fiddles[corner * 3 + 2]) + Mth.square(distanceY + this.fiddles[corner * 3 + 1])
                        + Mth.square(distanceX + this.fiddles[corner * 3]);
                if (minFiddledDistance > next) {
                    minI = i;
                    minFiddledDistance = next;
                }
            }

            int biomeX = (minI & 4) == 0 ? parentX : parentX + 1;
            int biomeY = (minI & 2) == 0 ? parentY : parentY + 1;
            int biomeZ = (minI & 1) == 0 ? parentZ : parentZ + 1;
            int corner = this.cornerIndex(biomeX, biomeY, biomeZ);
            if (this.cornerBiomeStamps[corner] == this.stamp) {
                return this.cornerBiomes[corner];
            }
            Biome biome = this.biomeManager.getNoiseBiomeAtQuart(biomeX, biomeY, biomeZ).value();
            this.cornerBiomes[corner] = biome;
            this.cornerBiomeStamps[corner] = this.stamp;
            return biome;
        }

        private int cornerIndex(int quartX, int quartY, int quartZ) {
            return ((quartY - this.cornerMinY) * this.cornersZ + (quartZ - this.cornerMinZ)) * this.cornersX + (quartX - this.cornerMinX);
        }

        /** The corner's index, with its offsets computed as {@code BiomeManager.getFiddledDistance} does */
        private int fiddle(int quartX, int quartY, int quartZ) {
            int corner = this.cornerIndex(quartX, quartY, quartZ);
            if (this.fiddleStamps[corner] != this.stamp) {
                long seed = this.zoomSeed;
                long rval = seed;
                rval = LinearCongruentialGenerator.next(rval, quartX);
                rval = LinearCongruentialGenerator.next(rval, quartY);
                rval = LinearCongruentialGenerator.next(rval, quartZ);
                rval = LinearCongruentialGenerator.next(rval, quartX);
                rval = LinearCongruentialGenerator.next(rval, quartY);
                rval = LinearCongruentialGenerator.next(rval, quartZ);
                this.fiddles[corner * 3] = fiddleOf(rval);
                rval = LinearCongruentialGenerator.next(rval, seed);
                this.fiddles[corner * 3 + 1] = fiddleOf(rval);
                rval = LinearCongruentialGenerator.next(rval, seed);
                this.fiddles[corner * 3 + 2] = fiddleOf(rval);
                this.fiddleStamps[corner] = this.stamp;
            }
            return corner;
        }

        private static double fiddleOf(long rval) {
            double uniform = Math.floorMod(rval >> 24, 1024) / 1024.0;
            return (uniform - 0.5) * 0.9;
        }

        Colors colors(ColorResolver resolver) {
            for (int i = 0; i < RESOLVER_SLOTS; i++) {
                Colors colors = this.slots[i];
                if (colors == null) {
                    return this.slots[i] = new Colors(resolver, this.biomes.length);
                }
                if (colors.resolver == resolver) {
                    return colors;
                }
            }
            // More resolvers than slots (mods can register their own): reuse the last slot
            Colors colors = new Colors(resolver, this.biomes.length);
            this.slots[RESOLVER_SLOTS - 1] = colors;
            return colors;
        }
    }

    /** One resolver's sample colours (grid entries) and blended tints (the section's blocks) */
    private static final class Colors {
        final ColorResolver resolver;
        int[] samples;
        int[] sampleStamps;
        final int[] blends = new int[16 * 16 * 16];
        final int[] blendStamps = new int[16 * 16 * 16];

        Colors(ColorResolver resolver, int entries) {
            this.resolver = resolver;
            this.resize(entries);
        }

        void resize(int entries) {
            this.samples = new int[entries];
            this.sampleStamps = new int[entries];
            Arrays.fill(this.blendStamps, 0);
        }

        void clearStamps() {
            Arrays.fill(this.sampleStamps, 0);
            Arrays.fill(this.blendStamps, 0);
        }
    }
}
