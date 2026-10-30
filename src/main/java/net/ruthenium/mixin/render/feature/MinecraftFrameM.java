package net.ruthenium.mixin.render.feature;

import net.minecraft.client.Minecraft;
import net.ruthenium.render.feature.SubmitPosePool;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/** The start of a frame, before anything is submitted: the submit poses of the last frame are free again */
@Mixin(Minecraft.class)
public class MinecraftFrameM {
    @Inject(method = "renderFrame", at = @At("HEAD"))
    private void beginFrame(boolean advanceGameTime, CallbackInfo ci) {
        SubmitPosePool.beginFrame();
    }
}
