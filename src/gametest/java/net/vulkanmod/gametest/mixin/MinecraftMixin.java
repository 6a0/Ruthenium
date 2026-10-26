package net.vulkanmod.gametest.mixin;

import net.minecraft.client.Minecraft;
import net.vulkanmod.gametest.FrameRecorder;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

@Mixin(Minecraft.class)
public class MinecraftMixin {

    @Inject(method = "renderFrame", at = @At("HEAD"))
    private void onFrameStart(boolean advanceGameTime, CallbackInfo ci) {
        FrameRecorder.onFrameStart((Minecraft) (Object) this);
    }
}
