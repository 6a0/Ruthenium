package net.vulkanmod.interfaces;

import net.minecraft.client.renderer.chunk.ChunkSectionLayer;
import net.minecraft.client.renderer.chunk.SectionRenderDispatcher;
import org.jspecify.annotations.Nullable;

/**
 * A compiled mesh's place in the terrain uber buffers per solid or cutout layer, cached on first use by
 * {@code TerrainDrawList}.
 */
public interface CachedSectionSlices {

    SectionRenderDispatcher.@Nullable RenderSectionBufferSlice vulkanmod$getSlice(ChunkSectionLayer layer);

    void vulkanmod$setSlice(ChunkSectionLayer layer, SectionRenderDispatcher.RenderSectionBufferSlice slice);
}
