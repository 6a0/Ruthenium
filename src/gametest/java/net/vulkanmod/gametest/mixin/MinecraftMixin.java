package net.vulkanmod.gametest.mixin;

import net.minecraft.client.Minecraft;
import net.vulkanmod.gametest.FrameRecorder;
import net.vulkanmod.gametest.GpuFrameProbe;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

@Mixin(Minecraft.class)
public class MinecraftMixin {

    @Inject(method = "renderFrame", at = @At("HEAD"))
    private void onFrameStart(boolean advanceGameTime, CallbackInfo ci) {
        FrameRecorder.onFrameStart((Minecraft) (Object) this);
        if (GpuFrameProbe.ENABLED) {
            GpuFrameProbe.frameStart();
        }
    }

    @Inject(method = "tick", at = @At("HEAD"))
    private void onTick(CallbackInfo ci) {
        FrameRecorder.onTick();
    }

    @Inject(method = "renderFrame", at = @At(value = "INVOKE", target = "Lcom/mojang/renderpearl/api/commands/CommandEncoder;submit()V"))
    private void beforeSubmit(boolean advanceGameTime, CallbackInfo ci) {
        if (GpuFrameProbe.ENABLED) {
            GpuFrameProbe.beforeSubmit();
        }
    }
}
