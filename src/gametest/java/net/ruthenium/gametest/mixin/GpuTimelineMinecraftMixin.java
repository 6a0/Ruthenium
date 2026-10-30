package net.ruthenium.gametest.mixin;

import net.minecraft.client.Minecraft;
import net.ruthenium.gametest.GpuTimelineProbe;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/** {@link GpuTimelineProbe}: frame start, acquire, blit, submit and present on the render thread */
@Mixin(Minecraft.class)
public class GpuTimelineMinecraftMixin {
    @Inject(method = "renderFrame", at = @At("HEAD"))
    private void timelineFrame(boolean advanceGameTime, CallbackInfo ci) {
        if (GpuTimelineProbe.ENABLED) {
            GpuTimelineProbe.frameStart();
        }
    }

    @Inject(method = "renderFrame", at = @At(value = "INVOKE", target = "Lcom/mojang/renderpearl/api/device/GpuSurface;acquireNextTexture()V"))
    private void timelineAcquire(boolean advanceGameTime, CallbackInfo ci) {
        GpuTimelineProbe.cpu(GpuTimelineProbe.C_ACQ0);
    }

    @Inject(method = "renderFrame", at = @At(value = "INVOKE", shift = At.Shift.AFTER, target = "Lcom/mojang/renderpearl/api/device/GpuSurface;acquireNextTexture()V"))
    private void timelineAcquired(boolean advanceGameTime, CallbackInfo ci) {
        GpuTimelineProbe.cpu(GpuTimelineProbe.C_ACQ1);
    }

    @Inject(method = "renderFrame", at = @At(value = "INVOKE", target = "Lcom/mojang/renderpearl/api/device/GpuSurface;blitFromTexture(Lcom/mojang/renderpearl/api/commands/CommandEncoder;Lcom/mojang/renderpearl/api/textures/GpuTextureView;)V"))
    private void timelineBlit(boolean advanceGameTime, CallbackInfo ci) {
        if (GpuTimelineProbe.ENABLED) {
            GpuTimelineProbe.cpu(GpuTimelineProbe.C_BLIT);
            GpuTimelineProbe.mark("blit");
        }
    }

    @Inject(method = "renderFrame", at = @At(value = "INVOKE", target = "Lcom/mojang/renderpearl/api/commands/CommandEncoder;submit()V"))
    private void timelineSubmit(boolean advanceGameTime, CallbackInfo ci) {
        if (GpuTimelineProbe.ENABLED) {
            GpuTimelineProbe.mark("submit");
            GpuTimelineProbe.cpu(GpuTimelineProbe.C_SUBMIT);
        }
    }

    @Inject(method = "renderFrame", at = @At(value = "INVOKE", target = "Lcom/mojang/renderpearl/api/device/GpuSurface;present()V"))
    private void timelinePresent(boolean advanceGameTime, CallbackInfo ci) {
        GpuTimelineProbe.cpu(GpuTimelineProbe.C_PRESENT0);
    }

    @Inject(method = "renderFrame", at = @At(value = "INVOKE", shift = At.Shift.AFTER, target = "Lcom/mojang/renderpearl/api/device/GpuSurface;present()V"))
    private void timelinePresented(boolean advanceGameTime, CallbackInfo ci) {
        GpuTimelineProbe.cpu(GpuTimelineProbe.C_PRESENT1);
        GpuTimelineProbe.frameEnd();
    }
}
