package net.ruthenium.mixin.render.chunk;

import com.llamalad7.mixinextras.injector.ModifyReturnValue;
import com.mojang.renderpearl.api.pipeline.RenderPipeline;
import com.mojang.renderpearl.api.vertex.VertexFormat;
import net.minecraft.client.renderer.chunk.ChunkSectionLayer;
import net.ruthenium.render.chunk.CompressedTerrain;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;

/** Solid and cutout sections use {@link CompressedTerrain}'s pipelines and vertex format while it is active */
@Mixin(ChunkSectionLayer.class)
public class ChunkSectionLayerM {

    @ModifyReturnValue(method = "pipeline", at = @At("RETURN"))
    private RenderPipeline compressedPipeline(RenderPipeline pipeline, boolean multiDraw) {
        return CompressedTerrain.pipeline((ChunkSectionLayer) (Object) this, multiDraw, pipeline);
    }

    @ModifyReturnValue(method = "vertexFormat", at = @At("RETURN"))
    private VertexFormat compressedFormat(VertexFormat format) {
        return CompressedTerrain.isActive((ChunkSectionLayer) (Object) this) ? CompressedTerrain.FORMAT : format;
    }
}
