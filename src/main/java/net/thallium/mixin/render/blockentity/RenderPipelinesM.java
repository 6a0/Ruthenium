package net.thallium.mixin.render.blockentity;

import com.mojang.renderpearl.api.pipeline.RenderPipeline;
import net.minecraft.client.renderer.RenderPipelines;
import net.thallium.render.blockentity.InstancedModels;
import net.thallium.render.entity.InstancedParts;
import net.thallium.render.blockentity.RetainedBlockEntities;
import net.thallium.render.chunk.CompressedTerrain;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

import java.util.ArrayList;
import java.util.List;

/**
 * Compiles {@link RetainedBlockEntities#PIPELINES}, {@link InstancedModels#PIPELINES} and {@link CompressedTerrain#PIPELINES}
 * with the other pipelines on resource reload; if one fails, that feature stays off
 */
@Mixin(RenderPipelines.class)
public class RenderPipelinesM {
    @Inject(method = "optionalPipelines", at = @At("RETURN"), cancellable = true)
    private static void addRetainedPipeline(CallbackInfoReturnable<List<RenderPipeline>> cir) {
        List<RenderPipeline> pipelines = new ArrayList<>(cir.getReturnValue());
        pipelines.addAll(RetainedBlockEntities.PIPELINES);
        pipelines.addAll(InstancedModels.PIPELINES);
        pipelines.addAll(InstancedParts.PIPELINES);
        pipelines.addAll(CompressedTerrain.PIPELINES);
        cir.setReturnValue(pipelines);
    }
}
