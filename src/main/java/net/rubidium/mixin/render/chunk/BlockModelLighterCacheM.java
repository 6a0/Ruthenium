package net.rubidium.mixin.render.chunk;

import net.minecraft.client.renderer.block.BlockAndTintGetter;
import net.minecraft.client.renderer.block.BlockModelLighter;
import net.minecraft.core.BlockPos;
import net.minecraft.util.LightCoordsUtil;
import net.minecraft.world.level.block.state.BlockState;
import net.rubidium.Initializer;
import net.rubidium.interfaces.SectionLightWindow;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Overwrite;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import java.util.Arrays;

/**
 * While a section compiles, the smooth lighting's light and shade values of the section and the blocks around it
 * are kept in arrays (18^3 entries) instead of vanilla's 100 entry FIFO maps, which a section's block order keeps
 * evicting. Each value is computed the same way, so the lighting is identical. Positions outside the section and
 * compiles without {@link #rubidium$beginSection} use vanilla's maps. One cache per thread, like vanilla's.
 */
@Mixin(BlockModelLighter.Cache.class)
public class BlockModelLighterCacheM implements SectionLightWindow {
    @Unique
    private static final int SIZE = 18;

    @Shadow private boolean enabled;
    @Shadow @Final private LightCoordsUtil.BrightnessGetter cachedBrightnessGetter;
    @Shadow @Final private it.unimi.dsi.fastutil.longs.Long2FloatLinkedOpenHashMap brightnessCache;

    // Allocated in code: array field initializers in mixins are not reliably merged (see CompiledSectionMeshM)
    @Unique private int[] rubidium$light;
    @Unique private int[] rubidium$lightStamps;
    @Unique private float[] rubidium$shade;
    @Unique private int[] rubidium$shadeStamps;
    /** An entry is valid when its stamp is this one, so a new section needs no clearing */
    @Unique private int rubidium$stamp;
    @Unique private boolean rubidium$active;
    @Unique private int rubidium$minX, rubidium$minY, rubidium$minZ;
    @Unique private LightCoordsUtil.BrightnessGetter rubidium$windowGetter;

    @Override
    public void rubidium$beginSection(BlockPos sectionOrigin) {
        if (!Initializer.CONFIG.sectionLightCache) {
            this.rubidium$active = false;
            return;
        }
        if (this.rubidium$light == null) {
            int entries = SIZE * SIZE * SIZE;
            this.rubidium$light = new int[entries];
            this.rubidium$lightStamps = new int[entries];
            this.rubidium$shade = new float[entries];
            this.rubidium$shadeStamps = new int[entries];
            this.rubidium$windowGetter = this::rubidium$packedBrightness;
        }
        if (++this.rubidium$stamp == 0) {
            Arrays.fill(this.rubidium$lightStamps, 0);
            Arrays.fill(this.rubidium$shadeStamps, 0);
            this.rubidium$stamp = 1;
        }
        this.rubidium$minX = sectionOrigin.getX() - 1;
        this.rubidium$minY = sectionOrigin.getY() - 1;
        this.rubidium$minZ = sectionOrigin.getZ() - 1;
        this.rubidium$active = true;
    }

    @Inject(method = "disable", at = @At("HEAD"))
    private void rubidium$endSection(CallbackInfo ci) {
        this.rubidium$active = false;
    }

    @Unique
    private int rubidium$index(BlockPos pos) {
        int x = pos.getX() - this.rubidium$minX;
        int y = pos.getY() - this.rubidium$minY;
        int z = pos.getZ() - this.rubidium$minZ;
        if ((x | y | z) < 0 || x >= SIZE || y >= SIZE || z >= SIZE) {
            return -1;
        }
        return (y * SIZE + z) * SIZE + x;
    }

    @Unique
    private int rubidium$packedBrightness(net.minecraft.world.level.BlockAndLightGetter level, BlockPos pos) {
        int index = this.rubidium$index(pos);
        if (index < 0) {
            return this.cachedBrightnessGetter.packedBrightness(level, pos);
        }
        if (this.rubidium$lightStamps[index] == this.rubidium$stamp) {
            return this.rubidium$light[index];
        }
        int value = LightCoordsUtil.BrightnessGetter.DEFAULT.packedBrightness(level, pos);
        this.rubidium$light[index] = value;
        this.rubidium$lightStamps[index] = this.rubidium$stamp;
        return value;
    }

    /**
     * @author Rubidium
     * @reason Section light window (see the class comment); otherwise vanilla's
     */
    @Overwrite
    public int getLightCoords(final BlockState state, final BlockAndTintGetter level, final BlockPos pos) {
        LightCoordsUtil.BrightnessGetter getter = !this.enabled ? LightCoordsUtil.BrightnessGetter.DEFAULT
                : this.rubidium$active ? this.rubidium$windowGetter : this.cachedBrightnessGetter;
        return LightCoordsUtil.getLightCoords(getter, level, state, pos);
    }

    /**
     * @author Rubidium
     * @reason Section light window (see the class comment); otherwise vanilla's
     */
    @Overwrite
    public float getShadeBrightness(final BlockState state, final BlockAndTintGetter level, final BlockPos pos) {
        if (this.enabled && this.rubidium$active) {
            int index = this.rubidium$index(pos);
            if (index >= 0) {
                if (this.rubidium$shadeStamps[index] == this.rubidium$stamp) {
                    return this.rubidium$shade[index];
                }
                float brightness = state.getShadeBrightness(level, pos);
                this.rubidium$shade[index] = brightness;
                this.rubidium$shadeStamps[index] = this.rubidium$stamp;
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
