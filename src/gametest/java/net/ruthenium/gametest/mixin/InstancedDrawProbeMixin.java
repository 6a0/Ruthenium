package net.ruthenium.gametest.mixin;

import com.mojang.renderpearl.api.commands.RenderPass;
import net.ruthenium.gametest.GpuPassProbe;
import net.ruthenium.render.blockentity.InstancedModels;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

@Mixin(value = InstancedModels.class, remap = false)
public class InstancedDrawProbeMixin {
    @Inject(method = "draw", at = @At("HEAD"))
    private static void stampStart(RenderPass pass, CallbackInfo ci) {
        GpuPassProbe.stamp(pass, 4);
        net.ruthenium.gametest.GpuTimelineProbe.markPass(pass, "instanced");
    }

    @Inject(method = "draw", at = @At("RETURN"))
    private static void stampEnd(RenderPass pass, CallbackInfo ci) {
        GpuPassProbe.stamp(pass, 5);
        net.ruthenium.gametest.GpuTimelineProbe.markPass(pass, "instancedEnd");
    }
}
