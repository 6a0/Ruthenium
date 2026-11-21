package dev.ruthenium.bench.mixin;

import dev.ruthenium.bench.Bench;
import net.minecraft.client.Minecraft;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/** The only hook: one call at the end of every frame (after present and the frame limiter). */
@Mixin(Minecraft.class)
public abstract class MinecraftMixin {
    @Inject(method = "renderFrame", at = @At("RETURN"))
    private void ruthenium$frameEnd(boolean advanceGameTime, CallbackInfo ci) {
        Bench.onFrameEnd((Minecraft) (Object) this);
    }
}
