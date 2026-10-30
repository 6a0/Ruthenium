package net.ruthenium.mixin.render;

import net.minecraft.client.Minecraft;
import net.ruthenium.render.FrameStats;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/** {@link FrameStats}: frame boundaries, client ticks, acquire, submit and present */
@Mixin(Minecraft.class)
public class MinecraftFrameStatsM {
    @Inject(method = "runTick", at = @At("HEAD"))
    private void ruthenium$frame(boolean advanceGameTime, CallbackInfo ci) {
        if (FrameStats.ENABLED) {
            FrameStats.frame();
        }
    }

    @Inject(method = "runTick", at = @At(value = "INVOKE", target = "Lnet/minecraft/client/Minecraft;tick()V"))
    private void ruthenium$tick(boolean advanceGameTime, CallbackInfo ci) {
        if (FrameStats.ENABLED) {
            FrameStats.begin(FrameStats.TICK);
        }
    }

    @Inject(method = "runTick", at = @At(value = "INVOKE", shift = At.Shift.AFTER, target = "Lnet/minecraft/client/Minecraft;tick()V"))
    private void ruthenium$ticked(boolean advanceGameTime, CallbackInfo ci) {
        if (FrameStats.ENABLED) {
            FrameStats.end(FrameStats.TICK);
        }
    }

    @Inject(method = "renderFrame", at = @At(value = "INVOKE", target = "Lcom/mojang/renderpearl/api/device/GpuSurface;acquireNextTexture()V"))
    private void ruthenium$acquire(boolean advanceGameTime, CallbackInfo ci) {
        if (FrameStats.ENABLED) {
            FrameStats.begin(FrameStats.ACQUIRE);
        }
    }

    @Inject(method = "renderFrame", at = @At(value = "INVOKE", shift = At.Shift.AFTER, target = "Lcom/mojang/renderpearl/api/device/GpuSurface;acquireNextTexture()V"))
    private void ruthenium$acquired(boolean advanceGameTime, CallbackInfo ci) {
        if (FrameStats.ENABLED) {
            FrameStats.end(FrameStats.ACQUIRE);
        }
    }

    @Inject(method = "renderFrame", at = @At(value = "INVOKE", target = "Lcom/mojang/renderpearl/api/commands/CommandEncoder;submit()V"))
    private void ruthenium$submit(boolean advanceGameTime, CallbackInfo ci) {
        if (FrameStats.ENABLED) {
            FrameStats.begin(FrameStats.SUBMIT);
        }
    }

    @Inject(method = "renderFrame", at = @At(value = "INVOKE", shift = At.Shift.AFTER, target = "Lcom/mojang/renderpearl/api/commands/CommandEncoder;submit()V"))
    private void ruthenium$submitted(boolean advanceGameTime, CallbackInfo ci) {
        if (FrameStats.ENABLED) {
            FrameStats.end(FrameStats.SUBMIT);
        }
    }

    @Inject(method = "renderFrame", at = @At(value = "INVOKE", target = "Lcom/mojang/renderpearl/api/device/GpuSurface;present()V"))
    private void ruthenium$present(boolean advanceGameTime, CallbackInfo ci) {
        if (FrameStats.ENABLED) {
            FrameStats.begin(FrameStats.PRESENT);
        }
    }

    @Inject(method = "renderFrame", at = @At(value = "INVOKE", shift = At.Shift.AFTER, target = "Lcom/mojang/renderpearl/api/device/GpuSurface;present()V"))
    private void ruthenium$presented(boolean advanceGameTime, CallbackInfo ci) {
        if (FrameStats.ENABLED) {
            FrameStats.end(FrameStats.PRESENT);
        }
    }
}
