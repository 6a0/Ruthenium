package net.ruthenium.gametest;

import net.fabricmc.fabric.api.client.gametest.v1.context.ClientGameTestContext;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.renderer.BiomeColors;
import net.minecraft.core.BlockPos;
import net.minecraft.world.level.ColorResolver;
import net.ruthenium.Initializer;
import net.ruthenium.render.chunk.BiomeTintBlend;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

/**
 * {@code -Pbench.tintCheck=true}: compares {@link BiomeTintBlend} with vanilla's blending for every block of the
 * sections around the player, for each vanilla resolver and blend radius 1, 2 and 7.
 */
final class BiomeTintCheck {
    static final boolean ENABLED = Boolean.parseBoolean(BenchmarkClientGameTest.prop("tintCheck", "false"));
    private static final Logger LOGGER = LoggerFactory.getLogger("Ruthenium Tint Check");
    private static final ColorResolver[] RESOLVERS = {BiomeColors.GRASS_COLOR_RESOLVER, BiomeColors.FOLIAGE_COLOR_RESOLVER,
            BiomeColors.DRY_FOLIAGE_COLOR_RESOLVER, BiomeColors.WATER_COLOR_RESOLVER};

    private BiomeTintCheck() {
    }

    static void run(ClientGameTestContext context) {
        context.runOnClient(mc -> {
            ClientLevel level = mc.level;
            int originalRadius = mc.options.biomeBlendRadius().get();
            boolean original = Initializer.CONFIG.biomeBlendCache;
            BlockPos center = mc.player.blockPosition();
            long checked = 0;
            int mismatches = 0;
            try {
                for (int radius : new int[]{1, 2, 7}) {
                    mc.options.biomeBlendRadius().set(radius);
                    // 12 x 12 sections around the player at 2 heights, block by block in the compile order
                    for (int sx = -6; sx < 6; sx++) {
                        for (int sz = -6; sz < 6; sz++) {
                            for (int sy : new int[]{(center.getY() >> 4) - 2, center.getY() >> 4}) {
                                int baseX = ((center.getX() >> 4) + sx) << 4;
                                int baseZ = ((center.getZ() >> 4) + sz) << 4;
                                for (BlockPos pos : BlockPos.betweenClosed(baseX, sy << 4, baseZ, baseX + 15, (sy << 4) + 15, baseZ + 15)) {
                                    for (ColorResolver resolver : RESOLVERS) {
                                        Initializer.CONFIG.biomeBlendCache = false;
                                        int expected = level.calculateBlockTint(pos, resolver);
                                        int actual = BiomeTintBlend.blend(level, pos, resolver, radius);
                                        checked++;
                                        if (expected != actual && mismatches++ < 10) {
                                            LOGGER.error("Tint mismatch at {} radius {}: vanilla {} cached {}", pos, radius,
                                                    Integer.toHexString(expected), Integer.toHexString(actual));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            } finally {
                mc.options.biomeBlendRadius().set(originalRadius);
                Initializer.CONFIG.biomeBlendCache = original;
            }
            LOGGER.info("Tint check: {} blends compared, {} mismatches", checked, mismatches);
            if (mismatches > 0) {
                throw new AssertionError(mismatches + " biome tint mismatches");
            }
        });
    }
}
