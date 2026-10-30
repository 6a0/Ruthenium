package net.ruthenium.world;

import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Holder;
import net.minecraft.core.QuartPos;
import net.minecraft.util.Mth;
import net.minecraft.util.RandomSource;
import net.minecraft.world.attribute.EnvironmentAttribute;
import net.minecraft.world.attribute.EnvironmentAttributeLayer;
import net.minecraft.world.attribute.EnvironmentAttributeSystem;
import net.minecraft.world.attribute.EnvironmentAttributes;
import net.minecraft.world.level.biome.Biome;
import net.minecraft.world.level.chunk.LevelChunk;
import net.minecraft.world.level.chunk.LevelChunkSection;
import net.minecraft.world.level.chunk.status.ChunkStatus;
import net.minecraft.world.level.levelgen.LegacyRandomSource;
import net.ruthenium.mixin.world.EnvironmentAttributeSystemAccessor;
import net.ruthenium.mixin.world.EnvironmentValueSamplerAccessor;
import net.ruthenium.mixin.world.LegacyRandomSourceAccessor;

import java.lang.ref.WeakReference;
import java.util.IdentityHashMap;
import java.util.Map;
import java.util.concurrent.atomic.AtomicLong;

/**
 * Faster {@code ClientLevel.doAnimateTick} (1334 random positions around the player every tick), with the same
 * results:
 * <ul>
 * <li>Its six {@code nextInt} calls per position on the level's {@link LegacyRandomSource} step the seed with plain
 * reads and writes instead of a compare-and-set each (the CAS only detects use from two threads at once; the
 * sequence is the same).</li>
 * <li>The ambient particles of a position ({@code environmentAttributes().getValue(AMBIENT_PARTICLES, pos)}, a
 * fuzzy-zoomed biome lookup) only depend on the position through the biome the zoom picks, one of the noise biomes
 * of the 2x2x2 quart corners around it. When the biome palettes of the chunk sections holding those corners contain
 * no biome with an ambient particle entry (vanilla: only four nether biomes have one), every candidate gives the
 * attribute's value without a biome modifier, which is computed once per call of {@code animateTick}. Only when the
 * attribute's sole positional layer is vanilla's biome layer.</li>
 * </ul>
 * Render thread only (animateTick is).
 */
public final class AnimateTick {
    private static final int SECTION_RANGE = 8;
    private static final int SECTION_SLOTS = SECTION_RANGE * SECTION_RANGE * SECTION_RANGE;

    /** Weak: a strong one kept the last level reachable after leaving it (its attribute layers capture its biome manager) */
    private static WeakReference<EnvironmentAttributeSystem> system = new WeakReference<>(null);
    private static boolean systemEligible;
    private static final Map<Biome, Boolean> AMBIENT_BIOMES = new IdentityHashMap<>();

    private static boolean eligible;
    private static Object noBiomeValue;
    private static int baseX, baseY, baseZ;
    private static int stamp;
    private static final int[] SECTION_STAMPS = new int[SECTION_SLOTS];
    private static final boolean[] SECTION_PLAIN = new boolean[SECTION_SLOTS];

    private AnimateTick() {
    }

    /** At the start of {@code animateTick(x, y, z)} */
    public static void begin(ClientLevel clientLevel, int x, int y, int z) {
        noBiomeValue = null;
        stamp++;
        if (stamp == 0) {
            java.util.Arrays.fill(SECTION_STAMPS, 0);
            stamp = 1;
        }
        // The positions are within 32 blocks (and the zoom 3 more) of (x, y, z): 6 sections per axis around it
        baseX = (x >> 4) - 3;
        baseY = (y >> 4) - 3;
        baseZ = (z >> 4) - 3;

        EnvironmentAttributeSystem current = clientLevel.environmentAttributes();
        if (current != system.get()) {
            system = new WeakReference<>(current);
            systemEligible = onlyBiomeLayerIsPositional(current, EnvironmentAttributes.AMBIENT_PARTICLES);
            AMBIENT_BIOMES.clear();
        }
        eligible = systemEligible;
    }

    public static int nextInt(RandomSource random, int bound) {
        if (random.getClass() != LegacyRandomSource.class || bound <= 0) {
            return random.nextInt(bound);
        }
        AtomicLong seed = ((LegacyRandomSourceAccessor) random).ruthenium$seed();
        long s = seed.getPlain();
        int result;
        if ((bound & bound - 1) == 0) {
            s = s * 25214903917L + 11L & 281474976710655L;
            result = (int) ((long) bound * (int) (s >> 17) >> 31);
        } else {
            int sample;
            do {
                s = s * 25214903917L + 11L & 281474976710655L;
                sample = (int) (s >> 17);
                result = sample % bound;
            } while (sample - result + (bound - 1) < 0);
        }
        seed.setPlain(s);
        return result;
    }

    @SuppressWarnings("unchecked")
    public static <V> V getValue(ClientLevel level, EnvironmentAttributeSystem attributes, EnvironmentAttribute<V> attribute, BlockPos pos) {
        if (!eligible || attribute != EnvironmentAttributes.AMBIENT_PARTICLES || attributes != system.get() || !noAmbientBiomesAround(level, pos)) {
            return attributes.getValue(attribute, pos);
        }
        if (noBiomeValue == null) {
            noBiomeValue = attributes.getValue(attribute, pos);
        }
        return (V) noBiomeValue;
    }

    /** Whether the sections of the noise biome corners {@code BiomeManager.getBiome(pos)} picks from hold no ambient biome */
    private static boolean noAmbientBiomesAround(ClientLevel level, BlockPos pos) {
        int qx0 = pos.getX() - 2 >> 2, qy0 = pos.getY() - 2 >> 2, qz0 = pos.getZ() - 2 >> 2;
        int cx0 = QuartPos.toSection(qx0), cx1 = QuartPos.toSection(qx0 + 1);
        int cz0 = QuartPos.toSection(qz0), cz1 = QuartPos.toSection(qz0 + 1);
        for (int cx = cx0; cx <= cx1; cx++) {
            for (int cz = cz0; cz <= cz1; cz++) {
                LevelChunk chunk = level.getChunkSource().getChunk(cx, cz, ChunkStatus.BIOMES, false);
                if (chunk == null) {
                    return false;
                }
                // ChunkAccess.getNoiseBiome clamps the quart y into the chunk
                int quartMinY = QuartPos.fromBlock(chunk.getMinY());
                int quartMaxY = quartMinY + QuartPos.fromBlock(chunk.getHeight()) - 1;
                int sy0 = chunk.getSectionIndex(QuartPos.toBlock(Mth.clamp(qy0, quartMinY, quartMaxY)));
                int sy1 = chunk.getSectionIndex(QuartPos.toBlock(Mth.clamp(qy0 + 1, quartMinY, quartMaxY)));
                for (int sy = sy0; sy <= sy1; sy++) {
                    if (!plainSection(chunk, cx, sy, cz)) {
                        return false;
                    }
                }
            }
        }
        return true;
    }

    private static boolean plainSection(LevelChunk chunk, int cx, int sectionIndex, int cz) {
        int rx = cx - baseX, ry = chunk.getSectionYFromSectionIndex(sectionIndex) - baseY, rz = cz - baseZ;
        boolean cacheable = rx >= 0 && rx < SECTION_RANGE && ry >= 0 && ry < SECTION_RANGE && rz >= 0 && rz < SECTION_RANGE;
        int slot = (rx * SECTION_RANGE + ry) * SECTION_RANGE + rz;
        if (cacheable && SECTION_STAMPS[slot] == stamp) {
            return SECTION_PLAIN[slot];
        }
        LevelChunkSection section = chunk.getSection(sectionIndex);
        boolean plain = !section.getBiomes().maybeHas(AnimateTick::hasAmbientParticles);
        if (cacheable) {
            SECTION_STAMPS[slot] = stamp;
            SECTION_PLAIN[slot] = plain;
        }
        return plain;
    }

    private static boolean hasAmbientParticles(Holder<Biome> biome) {
        Biome value = biome.value();
        Boolean has = AMBIENT_BIOMES.get(value);
        if (has == null) {
            has = value.getAttributes().contains(EnvironmentAttributes.AMBIENT_PARTICLES);
            AMBIENT_BIOMES.put(value, has);
        }
        return has;
    }

    /** Whether the attribute's only positional layer is the biome layer of {@code EnvironmentAttributeSystem} */
    private static boolean onlyBiomeLayerIsPositional(EnvironmentAttributeSystem attributes, EnvironmentAttribute<?> attribute) {
        try {
            Object sampler = ((EnvironmentAttributeSystemAccessor) attributes).ruthenium$attributeSamplers().get(attribute);
            if (sampler == null) {
                return false;
            }
            int positional = 0;
            for (Object layer : ((EnvironmentValueSamplerAccessor) sampler).ruthenium$layers()) {
                if (layer instanceof EnvironmentAttributeLayer.Positional<?>) {
                    if (!layer.getClass().getName().startsWith(EnvironmentAttributeSystem.class.getName() + "$$Lambda")) {
                        return false;
                    }
                    positional++;
                }
            }
            return positional == 1;
        } catch (RuntimeException e) {
            return false;
        }
    }

}
