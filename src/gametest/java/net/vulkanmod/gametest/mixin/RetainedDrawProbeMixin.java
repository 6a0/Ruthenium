package net.vulkanmod.gametest.mixin;

import com.mojang.renderpearl.api.commands.RenderPass;
import net.vulkanmod.gametest.GpuPassProbe;
import net.vulkanmod.render.blockentity.RetainedBlockEntities;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

@Mixin(value = RetainedBlockEntities.class, remap = false)
public class RetainedDrawProbeMixin {
    @Inject(method = "draw", at = @At("HEAD"))
    private static void stampStart(RenderPass pass, CallbackInfo ci) {
        GpuPassProbe.stamp(pass, 2);
    }

    @Inject(method = "draw", at = @At("RETURN"))
    private static void stampEnd(RenderPass pass, CallbackInfo ci) {
        GpuPassProbe.stamp(pass, 3);
    }
}
