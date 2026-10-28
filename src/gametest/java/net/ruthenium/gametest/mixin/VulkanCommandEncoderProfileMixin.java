package net.ruthenium.gametest.mixin;

import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.llamalad7.mixinextras.injector.wrapoperation.WrapOperation;
import com.mojang.renderpearl.backend.vulkan.VulkanCommandEncoder;
import net.minecraft.util.profiling.Profiler;
import net.minecraft.util.profiling.ProfilerFiller;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;

/** Splits vanilla's "submit" section: the wait for the submit two frames back (GPU) vs the rest */
@Mixin(VulkanCommandEncoder.class)
public class VulkanCommandEncoderProfileMixin {
    @WrapOperation(method = "submit", at = @At(value = "INVOKE",
            target = "Lcom/mojang/renderpearl/backend/vulkan/VulkanCommandEncoder;awaitSubmitCompletion(JJ)Z"))
    private boolean profileAwait(VulkanCommandEncoder encoder, long submitIndex, long timeout, Operation<Boolean> original) {
        ProfilerFiller profiler = Profiler.get();
        profiler.push("awaitGpu");
        boolean result = original.call(encoder, submitIndex, timeout);
        profiler.pop();
        return result;
    }

    @WrapOperation(method = "submit", at = @At(value = "INVOKE",
            target = "Lcom/mojang/renderpearl/backend/vulkan/VulkanCommandEncoder;endCommandBuffer()V"))
    private void profileEnd(VulkanCommandEncoder encoder, Operation<Void> original) {
        ProfilerFiller profiler = Profiler.get();
        profiler.push("endCommandBuffer");
        original.call(encoder);
        profiler.pop();
    }
}
