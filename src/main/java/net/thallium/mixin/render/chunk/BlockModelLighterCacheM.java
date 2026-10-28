package net.thallium.mixin.render.chunk;

import net.minecraft.client.renderer.block.BlockAndTintGetter;
import net.minecraft.client.renderer.block.BlockModelLighter;
import net.minecraft.core.BlockPos;
import net.minecraft.util.LightCoordsUtil;
import net.minecraft.world.level.block.state.BlockState;
import net.thallium.Initializer;
import net.thallium.interfaces.SectionLightWindow;
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
 * compiles without {@link #thallium$beginSection} use vanilla's maps. One cache per thread, like vanilla's.
 */
@Mixin(BlockModelLighter.Cache.class)
public class BlockModelLighterCacheM implements SectionLightWindow {
    @Unique
    private static final int SIZE = 18;

    @Shadow private boolean enabled;
    @Shadow @Final private LightCoordsUtil.BrightnessGetter cachedBrightnessGetter;
    @Shadow @Final private it.unimi.dsi.fastutil.longs.Long2FloatLinkedOpenHashMap brightnessCache;

    // Allocated in code: array field initializers in mixins are not reliably merged (see CompiledSectionMeshM)
    @Unique private int[] thallium$light;
    @Unique private int[] thallium$lightStamps;
    @Unique private float[] thallium$shade;
    @Unique private int[] thallium$shadeStamps;
    /** An entry is valid when its stamp is this one, so a new section needs no clearing */
    @Unique private int thallium$stamp;
    @Unique private boolean thallium$active;
    @Unique private int thallium$minX, thallium$minY, thallium$minZ;
    @Unique private LightCoordsUtil.BrightnessGetter thallium$windowGetter;

    @Override
    public void thallium$beginSection(BlockPos sectionOrigin) {
        if (!Initializer.CONFIG.sectionLightCache) {
            this.thallium$active = false;
            return;
        }
        if (this.thallium$light == null) {
            int entries = SIZE * SIZE * SIZE;
            this.thallium$light = new int[entries];
            this.thallium$lightStamps = new int[entries];
            this.thallium$shade = new float[entries];
            this.thallium$shadeStamps = new int[entries];
            this.thallium$windowGetter = this::thallium$packedBrightness;
        }
        if (++this.thallium$stamp == 0) {
            Arrays.fill(this.thallium$lightStamps, 0);
            Arrays.fill(this.thallium$shadeStamps, 0);
            this.thallium$stamp = 1;
        }
        this.thallium$minX = sectionOrigin.getX() - 1;
        this.thallium$minY = sectionOrigin.getY() - 1;
        this.thallium$minZ = sectionOrigin.getZ() - 1;
        this.thallium$active = true;
    }

    @Inject(method = "disable", at = @At("HEAD"))
    private void thallium$endSection(CallbackInfo ci) {
        this.thallium$active = false;
    }

    @Unique
    private int thallium$index(BlockPos pos) {
        int x = pos.getX() - this.thallium$minX;
        int y = pos.getY() - this.thallium$minY;
        int z = pos.getZ() - this.thallium$minZ;
        if ((x | y | z) < 0 || x >= SIZE || y >= SIZE || z >= SIZE) {
            return -1;
        }
        return (y * SIZE + z) * SIZE + x;
    }

    @Unique
    private int thallium$packedBrightness(net.minecraft.world.level.BlockAndLightGetter level, BlockPos pos) {
        int index = this.thallium$index(pos);
        if (index < 0) {
            return this.cachedBrightnessGetter.packedBrightness(level, pos);
        }
        if (this.thallium$lightStamps[index] == this.thallium$stamp) {
            return this.thallium$light[index];
        }
        int value = LightCoordsUtil.BrightnessGetter.DEFAULT.packedBrightness(level, pos);
        this.thallium$light[index] = value;
        this.thallium$lightStamps[index] = this.thallium$stamp;
        return value;
    }

    /**
     * @author Thallium
     * @reason Section light window (see the class comment); otherwise vanilla's
     */
    @Overwrite
    public int getLightCoords(final BlockState state, final BlockAndTintGetter level, final BlockPos pos) {
        LightCoordsUtil.BrightnessGetter getter = !this.enabled ? LightCoordsUtil.BrightnessGetter.DEFAULT
                : this.thallium$active ? this.thallium$windowGetter : this.cachedBrightnessGetter;
        return LightCoordsUtil.getLightCoords(getter, level, state, pos);
    }

    /**
     * @author Thallium
     * @reason Section light window (see the class comment); otherwise vanilla's
     */
    @Overwrite
    public float getShadeBrightness(final BlockState state, final BlockAndTintGetter level, final BlockPos pos) {
        if (this.enabled && this.thallium$active) {
            int index = this.thallium$index(pos);
            if (index >= 0) {
                if (this.thallium$shadeStamps[index] == this.thallium$stamp) {
                    return this.thallium$shade[index];
                }
                float brightness = state.getShadeBrightness(level, pos);
                this.thallium$shade[index] = brightness;
                this.thallium$shadeStamps[index] = this.thallium$stamp;
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
