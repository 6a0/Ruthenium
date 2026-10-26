package net.vulkanmod.mixin.render.blockentity;

import com.mojang.renderpearl.api.pipeline.RenderPipeline;
import net.minecraft.client.renderer.RenderPipelines;
import net.vulkanmod.render.blockentity.InstancedModels;
import net.vulkanmod.render.blockentity.RetainedChests;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

import java.util.ArrayList;
import java.util.List;

/**
 * Compiles {@link RetainedChests#PIPELINE} and {@link InstancedModels#PIPELINES} with the other pipelines on resource
 * reload; if one fails, that feature stays off
 */
@Mixin(RenderPipelines.class)
public class RenderPipelinesM {
    @Inject(method = "optionalPipelines", at = @At("RETURN"), cancellable = true)
    private static void addRetainedPipeline(CallbackInfoReturnable<List<RenderPipeline>> cir) {
        List<RenderPipeline> pipelines = new ArrayList<>(cir.getReturnValue());
        pipelines.add(RetainedChests.PIPELINE);
        pipelines.addAll(InstancedModels.PIPELINES);
        cir.setReturnValue(pipelines);
    }
}
