package net.rubidium.interfaces;

import net.minecraft.client.renderer.chunk.ChunkSectionLayer;
import net.rubidium.render.chunk.SectionFaceGroups;
import org.jspecify.annotations.Nullable;

/**
 * Per-layer face groups of a section's compile results and of the compiled mesh made from them
 * ({@link SectionFaceGroups}).
 */
public interface FaceGroupsHolder {

    /** @return the layer's groups, or null if its quads were not grouped */
    @Nullable SectionFaceGroups rubidium$getFaceGroups(ChunkSectionLayer layer);

    void rubidium$setFaceGroups(ChunkSectionLayer layer, @Nullable SectionFaceGroups groups);
}
