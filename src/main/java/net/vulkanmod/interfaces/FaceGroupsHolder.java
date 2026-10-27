package net.vulkanmod.interfaces;

import net.minecraft.client.renderer.chunk.ChunkSectionLayer;
import net.vulkanmod.render.chunk.SectionFaceGroups;
import org.jspecify.annotations.Nullable;

/**
 * Per-layer face groups of a section's compile results and of the compiled mesh made from them
 * ({@link SectionFaceGroups}).
 */
public interface FaceGroupsHolder {

    /** @return the layer's groups, or null if its quads were not grouped */
    @Nullable SectionFaceGroups vulkanmod$getFaceGroups(ChunkSectionLayer layer);

    void vulkanmod$setFaceGroups(ChunkSectionLayer layer, @Nullable SectionFaceGroups groups);
}
