package net.ruthenium.gametest.mixin;

import com.mojang.renderpearl.api.commands.RenderPass;
import net.ruthenium.gametest.GpuPassProbe;
import net.ruthenium.render.blockentity.RetainedBlockEntities;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

@Mixin(value = RetainedBlockEntities.class, remap = false)
public class RetainedDrawProbeMixin {
    @Inject(method = "draw", at = @At("HEAD"))
    private static void stampStart(RenderPass pass, CallbackInfo ci) {
        GpuPassProbe.stamp(pass, 2);
        net.ruthenium.gametest.GpuTimelineProbe.markPass(pass, "retained");
    }

    @Inject(method = "draw", at = @At("RETURN"))
    private static void stampEnd(RenderPass pass, CallbackInfo ci) {
        GpuPassProbe.stamp(pass, 3);
        net.ruthenium.gametest.GpuTimelineProbe.markPass(pass, "retainedEnd");
    }
}
