package net.ruthenium.gametest.mixin;

import net.minecraft.util.profiling.Profiler;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/**
 * Splits vanilla's "submit" section on the OpenGL backend like {@link VulkanCommandEncoderProfileMixin} does on Vulkan: the
 * fence of this submit ({@code glFenceSync}), the wait for the submit two frames back ({@code glClientWaitSync}, the GPU)
 * and the transient memory's rotation
 */
@Mixin(targets = "com.mojang.renderpearl.backend.opengl.GlCommandEncoder")
public class GlCommandEncoderProfileMixin {
    @Inject(method = "submit", at = @At("HEAD"))
    private void profileFenceStart(CallbackInfo ci) {
        Profiler.get().push("fenceSync");
    }

    @Inject(method = "submit", at = @At(value = "INVOKE",
            target = "Lcom/mojang/renderpearl/backend/opengl/GlCommandEncoder;awaitSubmit(JJ)Z"))
    private void profileAwaitStart(CallbackInfo ci) {
        Profiler.get().pop();
        Profiler.get().push("awaitGpu");
    }

    @Inject(method = "submit", at = @At(value = "INVOKE",
            target = "Lcom/mojang/renderpearl/backend/opengl/GlTransientMemory;rotate()V"))
    private void profileTransientStart(CallbackInfo ci) {
        Profiler.get().pop();
        Profiler.get().push("transientRotate");
    }

    @Inject(method = "submit", at = @At("RETURN"))
    private void profileEnd(CallbackInfo ci) {
        Profiler.get().pop();
    }
}
