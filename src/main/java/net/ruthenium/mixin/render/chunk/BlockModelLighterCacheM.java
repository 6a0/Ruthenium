package net.ruthenium.mixin.render.chunk;

import net.minecraft.client.renderer.block.BlockAndTintGetter;
import net.minecraft.client.renderer.block.BlockModelLighter;
import net.minecraft.core.BlockPos;
import net.minecraft.util.LightCoordsUtil;
import net.minecraft.world.level.block.state.BlockState;
import net.ruthenium.Initializer;
import net.ruthenium.interfaces.SectionLightWindow;
import net.ruthenium.render.chunk.SectionScratch;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Overwrite;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/**
 * While a section compiles, the smooth lighting's light and shade values of the section and the blocks around it
 * are kept in arrays (18^3 entries) instead of vanilla's 100 entry FIFO maps, which a section's block order keeps
 * evicting. Each value is computed the same way, so the lighting is identical. Positions outside the section and
 * compiles without {@link #ruthenium$beginSection} use vanilla's maps. One cache per thread, like vanilla's; the
 * arrays belong to the compile's scratch ({@link SectionScratch}) and are only referenced here while it compiles.
 */
@Mixin(BlockModelLighter.Cache.class)
public class BlockModelLighterCacheM implements SectionLightWindow {
    @Unique
    private static final int SIZE = SectionScratch.WINDOW;

    @Shadow private boolean enabled;
    @Shadow @Final private LightCoordsUtil.BrightnessGetter cachedBrightnessGetter;
    @Shadow @Final private it.unimi.dsi.fastutil.longs.Long2FloatLinkedOpenHashMap brightnessCache;

    // The window of the compile's scratch, set from beginSection to disable (the end of the compile)
    @Unique private int[] ruthenium$light;
    @Unique private byte[] ruthenium$lightStamps;
    @Unique private float[] ruthenium$shade;
    @Unique private byte[] ruthenium$shadeStamps;
    /** An entry is valid when its stamp is this one, so a new section needs no clearing */
    @Unique private byte ruthenium$stamp;
    @Unique private boolean ruthenium$active;
    @Unique private int ruthenium$minX, ruthenium$minY, ruthenium$minZ;
    @Unique private LightCoordsUtil.BrightnessGetter ruthenium$windowGetter;

    @Override
    public void ruthenium$beginSection(BlockPos sectionOrigin, SectionScratch scratch) {
        if (!Initializer.CONFIG.sectionLightCache) {
            this.ruthenium$active = false;
            return;
        }
        if (this.ruthenium$windowGetter == null) {
            this.ruthenium$windowGetter = this::ruthenium$packedBrightness;
        }
        SectionScratch.Light window = scratch.newLight();
        this.ruthenium$light = window.light;
        this.ruthenium$lightStamps = window.lightStamps;
        this.ruthenium$shade = window.shade;
        this.ruthenium$shadeStamps = window.shadeStamps;
        this.ruthenium$stamp = window.stamp;
        this.ruthenium$minX = sectionOrigin.getX() - 1;
        this.ruthenium$minY = sectionOrigin.getY() - 1;
        this.ruthenium$minZ = sectionOrigin.getZ() - 1;
        this.ruthenium$active = true;
    }

    @Inject(method = "disable", at = @At("HEAD"))
    private void ruthenium$endSection(CallbackInfo ci) {
        this.ruthenium$active = false;
        // The scratch goes back to the pool after this: keep no reference to its arrays
        this.ruthenium$light = null;
        this.ruthenium$lightStamps = null;
        this.ruthenium$shade = null;
        this.ruthenium$shadeStamps = null;
    }

    @Unique
    private int ruthenium$index(BlockPos pos) {
        int x = pos.getX() - this.ruthenium$minX;
        int y = pos.getY() - this.ruthenium$minY;
        int z = pos.getZ() - this.ruthenium$minZ;
        if ((x | y | z) < 0 || x >= SIZE || y >= SIZE || z >= SIZE) {
            return -1;
        }
        return (y * SIZE + z) * SIZE + x;
    }

    @Unique
    private int ruthenium$packedBrightness(net.minecraft.world.level.BlockAndLightGetter level, BlockPos pos) {
        int index = this.ruthenium$index(pos);
        if (index < 0) {
            return this.cachedBrightnessGetter.packedBrightness(level, pos);
        }
        if (this.ruthenium$lightStamps[index] == this.ruthenium$stamp) {
            return this.ruthenium$light[index];
        }
        int value = LightCoordsUtil.BrightnessGetter.DEFAULT.packedBrightness(level, pos);
        this.ruthenium$light[index] = value;
        this.ruthenium$lightStamps[index] = this.ruthenium$stamp;
        return value;
    }

    /**
     * @author Ruthenium
     * @reason Section light window (see the class comment); otherwise vanilla's
     */
    @Overwrite
    public int getLightCoords(final BlockState state, final BlockAndTintGetter level, final BlockPos pos) {
        LightCoordsUtil.BrightnessGetter getter = !this.enabled ? LightCoordsUtil.BrightnessGetter.DEFAULT
                : this.ruthenium$active ? this.ruthenium$windowGetter : this.cachedBrightnessGetter;
        return LightCoordsUtil.getLightCoords(getter, level, state, pos);
    }

    /**
     * @author Ruthenium
     * @reason Section light window (see the class comment); otherwise vanilla's
     */
    @Overwrite
    public float getShadeBrightness(final BlockState state, final BlockAndTintGetter level, final BlockPos pos) {
        if (this.enabled && this.ruthenium$active) {
            int index = this.ruthenium$index(pos);
            if (index >= 0) {
                if (this.ruthenium$shadeStamps[index] == this.ruthenium$stamp) {
                    return this.ruthenium$shade[index];
                }
                float brightness = state.getShadeBrightness(level, pos);
                this.ruthenium$shade[index] = brightness;
                this.ruthenium$shadeStamps[index] = this.ruthenium$stamp;
                return brightness;
            }
        }

        long key = pos.asLong();
        if (this.enabled) {
            float cached = this.brightnessCache.get(key);
            if (!Float.isNaN(cached)) {
                return cached;
            }
        }

        float brightness = state.getShadeBrightness(level, pos);
        if (this.enabled) {
            if (this.brightnessCache.size() == 100) {
                this.brightnessCache.removeFirstFloat();
            }

            this.brightnessCache.put(key, brightness);
        }

        return brightness;
    }
}
