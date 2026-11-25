package dev.ruthenium.bench.mixin;

import net.minecraft.client.renderer.WeatherEffectRenderer;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.ModifyVariable;

/**
 * For a picture of rain or snow to hold another client's against: with
 * {@code ruthenium.bench.weatherTime} set, the columns' pictures are scrolled to where they are at
 * that game time and no part of a tick, whatever the level's time is.
 */
@Mixin(WeatherEffectRenderer.class)
public abstract class WeatherEffectRendererMixin {
    @ModifyVariable(method = {"createRainColumnInstance", "createSnowColumnInstance"}, at = @At("HEAD"), argsOnly = true)
    private long ruthenium$fixedTime(long ticks) {
        String time = System.getProperty("ruthenium.bench.weatherTime");
        return time == null ? ticks : Long.parseLong(time);
    }

    @ModifyVariable(method = {"createRainColumnInstance", "createSnowColumnInstance"}, at = @At("HEAD"), argsOnly = true)
    private float ruthenium$noPartialTick(float partialTicks) {
        return System.getProperty("ruthenium.bench.weatherTime") == null ? partialTicks : 0.0F;
    }
}
