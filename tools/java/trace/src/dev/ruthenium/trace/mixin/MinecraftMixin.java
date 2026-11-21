package dev.ruthenium.trace.mixin;

import dev.ruthenium.trace.Recorder;
import net.minecraft.client.Minecraft;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/** Frame and tick boundaries. */
@Mixin(Minecraft.class)
public abstract class MinecraftMixin {
    @Inject(method = "runTick", at = @At("HEAD"))
    private void ruthenium$frameStart(boolean advanceGameTime, CallbackInfo ci) {
        Recorder.frameStart();
    }

    @Inject(method = "tick", at = @At("HEAD"))
    private void ruthenium$tickStart(CallbackInfo ci) {
        Recorder.tickStart((Minecraft) (Object) this);
    }

    @Inject(method = "tick", at = @At("TAIL"))
    private void ruthenium$tickEnd(CallbackInfo ci) {
        Recorder.tickEnd((Minecraft) (Object) this);
    }
}
