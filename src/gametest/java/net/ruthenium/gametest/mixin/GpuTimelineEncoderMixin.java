package net.ruthenium.gametest.mixin;

import com.mojang.renderpearl.api.commands.RenderPassDescriptor;
import com.mojang.renderpearl.frontend.FrontendCommandEncoder;
import net.ruthenium.gametest.GpuTimelineProbe;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

/** {@link GpuTimelineProbe}: a GPU mark before every render pass begins and after it ends */
@Mixin(value = FrontendCommandEncoder.class, remap = false)
public class GpuTimelineEncoderMixin {
    @Unique
    private String timelineLabel;

    @Inject(method = "createRenderPass(Lcom/mojang/renderpearl/api/commands/RenderPassDescriptor;)Lcom/mojang/renderpearl/api/commands/RenderPass;", at = @At("HEAD"))
    private void timelineBegin(RenderPassDescriptor descriptor, CallbackInfoReturnable<?> cir) {
        if (GpuTimelineProbe.PASSES) {
            timelineLabel = descriptor.label().get();
            GpuTimelineProbe.mark("<" + timelineLabel);
        }
    }

    @Inject(method = "submitRenderPass", at = @At("TAIL"))
    private void timelineEnd(CallbackInfo ci) {
        if (GpuTimelineProbe.PASSES) {
            GpuTimelineProbe.mark(timelineLabel + ">");
        }
    }
}
