package net.ruthenium.gametest.mixin;

import net.minecraft.client.Minecraft;
import net.ruthenium.gametest.FrameRecorder;
import net.ruthenium.gametest.GpuFrameProbe;
import net.ruthenium.gametest.TickProbe;
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
        TickProbe.start = System.nanoTime();
    }

    @Inject(method = "tick", at = @At("RETURN"))
    private void afterTick(CallbackInfo ci) {
        TickProbe.endTick(System.nanoTime() - TickProbe.start);
    }

    @Inject(method = "renderFrame", at = @At(value = "INVOKE", target = "Lcom/mojang/renderpearl/api/commands/CommandEncoder;submit()V"))
    private void beforeSubmit(boolean advanceGameTime, CallbackInfo ci) {
        if (GpuFrameProbe.ENABLED) {
            GpuFrameProbe.beforeSubmit();
        }
    }
}
