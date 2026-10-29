package net.ruthenium.gametest.mixin;

import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.llamalad7.mixinextras.injector.wrapoperation.WrapOperation;
import com.mojang.renderpearl.backend.vulkan.DestructionQueue;
import com.mojang.renderpearl.backend.vulkan.VulkanCommandEncoder;
import com.mojang.renderpearl.backend.vulkan.VulkanCommandPool;
import com.mojang.renderpearl.backend.vulkan.VulkanQueue;
import com.mojang.renderpearl.backend.vulkan.VulkanTransientMemory;
import net.minecraft.util.profiling.Profiler;
import net.minecraft.util.profiling.ProfilerFiller;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;

/**
 * Splits vanilla's "submit" section: the wait for the submit two frames back (GPU), ending the command buffer, the queue
 * submit ({@code vkQueueSubmit2}), the destruction queue, the command pool reset and the transient memory
 */
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

    @WrapOperation(method = "submit", at = @At(value = "INVOKE", target = "Lcom/mojang/renderpearl/backend/vulkan/VulkanQueue$Submission;close()V"))
    private void profileQueueSubmit(VulkanQueue.Submission s, Operation<Void> original) {
        Profiler.get().push("queueSubmit");
        original.call(s);
        Profiler.get().pop();
    }

    @WrapOperation(method = "submit", at = @At(value = "INVOKE", target = "Lcom/mojang/renderpearl/backend/vulkan/DestructionQueue;rotate()Z"))
    private boolean profileDestroyQueue(DestructionQueue<?> q, Operation<Boolean> original) {
        Profiler.get().push("destroyQueue");
        boolean r = original.call(q);
        Profiler.get().pop();
        return r;
    }

    @WrapOperation(method = "submit", at = @At(value = "INVOKE", target = "Lcom/mojang/renderpearl/backend/vulkan/VulkanCommandPool;reset()V"))
    private void profilePoolReset(VulkanCommandPool p, Operation<Void> original) {
        Profiler.get().push("poolReset");
        original.call(p);
        Profiler.get().pop();
    }

    @WrapOperation(method = "submit", at = @At(value = "INVOKE", target = "Lcom/mojang/renderpearl/backend/vulkan/VulkanTransientMemory;endSubmit()V"))
    private void profileTransientEnd(VulkanTransientMemory m, Operation<Void> original) {
        Profiler.get().push("transientEnd");
        original.call(m);
        Profiler.get().pop();
    }

    @WrapOperation(method = "submit", at = @At(value = "INVOKE", target = "Lcom/mojang/renderpearl/backend/vulkan/VulkanTransientMemory;beginSubmit()V"))
    private void profileTransientBegin(VulkanTransientMemory m, Operation<Void> original) {
        Profiler.get().push("transientBegin");
        original.call(m);
        Profiler.get().pop();
    }
}
