package dev.ruthenium.bench.mixin;

import net.minecraft.client.renderer.CloudRenderer;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.ModifyVariable;

/**
 * For a picture of the clouds to hold another client's against: with
 * {@code ruthenium.bench.cloudTime} set, the clouds are drawn where they are at that game time and
 * no part of a tick, whatever the level's time is.
 */
@Mixin(CloudRenderer.class)
public abstract class CloudRendererMixin {
    @ModifyVariable(method = "prepare(ILnet/minecraft/client/CloudStatus;FILnet/minecraft/world/phys/Vec3;JF)V", at = @At("HEAD"), argsOnly = true)
    private long ruthenium$fixedTime(long gameTime) {
        String time = System.getProperty("ruthenium.bench.cloudTime");
        return time == null ? gameTime : Long.parseLong(time);
    }

    @ModifyVariable(method = "prepare(ILnet/minecraft/client/CloudStatus;FILnet/minecraft/world/phys/Vec3;JF)V", at = @At("HEAD"), argsOnly = true, ordinal = 1)
    private float ruthenium$noPartialTick(float partialTicks) {
        return System.getProperty("ruthenium.bench.cloudTime") == null ? partialTicks : 0.0F;
    }
}
