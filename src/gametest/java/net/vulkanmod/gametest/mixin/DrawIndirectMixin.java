package net.vulkanmod.gametest.mixin;

import com.mojang.renderpearl.api.buffers.GpuBuffer;
import com.mojang.renderpearl.api.commands.RenderPass;
import com.mojang.renderpearl.api.pipeline.IndexType;
import com.mojang.renderpearl.api.pipeline.RenderPipeline;
import net.minecraft.client.renderer.chunk.ChunkSectionLayer;
import net.minecraft.client.renderer.chunk.ChunkSectionsToRender;
import net.vulkanmod.gametest.GpuProbe;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

@Mixin(ChunkSectionsToRender.DrawIndirect.class)
public class DrawIndirectMixin {

    @Inject(method = "render", at = @At("HEAD"))
    private void beforeLayer(ChunkSectionLayer layer, RenderPass renderPass, GpuBuffer defaultIndexBuffer, IndexType defaultIndexType,
                             RenderPipeline override, RenderPipeline overrideMultidraw, CallbackInfo ci) {
        if (GpuProbe.ENABLED) {
            GpuProbe.beforeLayer(layer, renderPass);
        }
    }

    @Inject(method = "render", at = @At("RETURN"))
    private void afterLayer(ChunkSectionLayer layer, RenderPass renderPass, GpuBuffer defaultIndexBuffer, IndexType defaultIndexType,
                            RenderPipeline override, RenderPipeline overrideMultidraw, CallbackInfo ci) {
        if (GpuProbe.ENABLED) {
            GpuProbe.afterLayer(layer, renderPass);
        }
    }
}
