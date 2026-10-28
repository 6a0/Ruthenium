package net.thallium.mixin.render.chunk;

import com.llamalad7.mixinextras.sugar.Local;
import com.mojang.renderpearl.api.pipeline.RenderPipeline;
import net.minecraft.client.renderer.chunk.ChunkSectionLayer;
import net.minecraft.client.renderer.chunk.ChunkSectionsToRender;
import net.thallium.render.chunk.CompressedTerrain;
import org.jspecify.annotations.Nullable;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.ModifyVariable;

/** The wireframe debug view of compressed layers needs the compressed wireframe pipelines */
@Mixin({ChunkSectionsToRender.DrawIndirect.class, ChunkSectionsToRender.DrawSeparate.class})
public class ChunkSectionsToRenderM {

    @ModifyVariable(method = "render", at = @At("HEAD"), argsOnly = true, ordinal = 0)
    private @Nullable RenderPipeline compressedOverride(@Nullable RenderPipeline override, @Local(argsOnly = true) ChunkSectionLayer layer) {
        return override != null ? CompressedTerrain.wireframe(layer, override) : null;
    }

    @ModifyVariable(method = "render", at = @At("HEAD"), argsOnly = true, ordinal = 1)
    private @Nullable RenderPipeline compressedMultidrawOverride(@Nullable RenderPipeline override, @Local(argsOnly = true) ChunkSectionLayer layer) {
        return override != null ? CompressedTerrain.wireframe(layer, override) : null;
    }
}
