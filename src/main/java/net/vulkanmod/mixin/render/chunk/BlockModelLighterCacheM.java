package net.vulkanmod.mixin.render.chunk;

import net.minecraft.client.renderer.block.BlockAndTintGetter;
import net.minecraft.client.renderer.block.BlockModelLighter;
import net.minecraft.core.BlockPos;
import net.minecraft.util.LightCoordsUtil;
import net.minecraft.world.level.block.state.BlockState;
import net.vulkanmod.Initializer;
import net.vulkanmod.interfaces.SectionLightWindow;
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
 * compiles without {@link #vulkanmod$beginSection} use vanilla's maps. One cache per thread, like vanilla's.
 */
@Mixin(BlockModelLighter.Cache.class)
public class BlockModelLighterCacheM implements SectionLightWindow {
    @Unique
    private static final int SIZE = 18;

    @Shadow private boolean enabled;
    @Shadow @Final private LightCoordsUtil.BrightnessGetter cachedBrightnessGetter;
    @Shadow @Final private it.unimi.dsi.fastutil.longs.Long2FloatLinkedOpenHashMap brightnessCache;

    // Allocated in code: array field initializers in mixins are not reliably merged (see CompiledSectionMeshM)
    @Unique private int[] vulkanmod$light;
    @Unique private int[] vulkanmod$lightStamps;
    @Unique private float[] vulkanmod$shade;
    @Unique private int[] vulkanmod$shadeStamps;
    /** An entry is valid when its stamp is this one, so a new section needs no clearing */
    @Unique private int vulkanmod$stamp;
    @Unique private boolean vulkanmod$active;
    @Unique private int vulkanmod$minX, vulkanmod$minY, vulkanmod$minZ;
    @Unique private LightCoordsUtil.BrightnessGetter vulkanmod$windowGetter;

    @Override
    public void vulkanmod$beginSection(BlockPos sectionOrigin) {
        if (!Initializer.CONFIG.sectionLightCache) {
            this.vulkanmod$active = false;
            return;
        }
        if (this.vulkanmod$light == null) {
            int entries = SIZE * SIZE * SIZE;
            this.vulkanmod$light = new int[entries];
            this.vulkanmod$lightStamps = new int[entries];
            this.vulkanmod$shade = new float[entries];
            this.vulkanmod$shadeStamps = new int[entries];
            this.vulkanmod$windowGetter = this::vulkanmod$packedBrightness;
        }
        if (++this.vulkanmod$stamp == 0) {
            Arrays.fill(this.vulkanmod$lightStamps, 0);
            Arrays.fill(this.vulkanmod$shadeStamps, 0);
            this.vulkanmod$stamp = 1;
        }
        this.vulkanmod$minX = sectionOrigin.getX() - 1;
        this.vulkanmod$minY = sectionOrigin.getY() - 1;
        this.vulkanmod$minZ = sectionOrigin.getZ() - 1;
        this.vulkanmod$active = true;
    }

    @Inject(method = "disable", at = @At("HEAD"))
    private void vulkanmod$endSection(CallbackInfo ci) {
        this.vulkanmod$active = false;
    }

    @Unique
    private int vulkanmod$index(BlockPos pos) {
        int x = pos.getX() - this.vulkanmod$minX;
        int y = pos.getY() - this.vulkanmod$minY;
        int z = pos.getZ() - this.vulkanmod$minZ;
        if ((x | y | z) < 0 || x >= SIZE || y >= SIZE || z >= SIZE) {
            return -1;
        }
        return (y * SIZE + z) * SIZE + x;
    }

    @Unique
    private int vulkanmod$packedBrightness(net.minecraft.world.level.BlockAndLightGetter level, BlockPos pos) {
        int index = this.vulkanmod$index(pos);
        if (index < 0) {
            return this.cachedBrightnessGetter.packedBrightness(level, pos);
        }
        if (this.vulkanmod$lightStamps[index] == this.vulkanmod$stamp) {
            return this.vulkanmod$light[index];
        }
        int value = LightCoordsUtil.BrightnessGetter.DEFAULT.packedBrightness(level, pos);
        this.vulkanmod$light[index] = value;
        this.vulkanmod$lightStamps[index] = this.vulkanmod$stamp;
        return value;
    }

    /**
     * @author VulkanMod
     * @reason Section light window (see the class comment); otherwise vanilla's
     */
    @Overwrite
    public int getLightCoords(final BlockState state, final BlockAndTintGetter level, final BlockPos pos) {
        LightCoordsUtil.BrightnessGetter getter = !this.enabled ? LightCoordsUtil.BrightnessGetter.DEFAULT
                : this.vulkanmod$active ? this.vulkanmod$windowGetter : this.cachedBrightnessGetter;
        return LightCoordsUtil.getLightCoords(getter, level, state, pos);
    }

    /**
     * @author VulkanMod
     * @reason Section light window (see the class comment); otherwise vanilla's
     */
    @Overwrite
    public float getShadeBrightness(final BlockState state, final BlockAndTintGetter level, final BlockPos pos) {
        if (this.enabled && this.vulkanmod$active) {
            int index = this.vulkanmod$index(pos);
            if (index >= 0) {
                if (this.vulkanmod$shadeStamps[index] == this.vulkanmod$stamp) {
                    return this.vulkanmod$shade[index];
                }
                float brightness = state.getShadeBrightness(level, pos);
                this.vulkanmod$shade[index] = brightness;
                this.vulkanmod$shadeStamps[index] = this.vulkanmod$stamp;
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
