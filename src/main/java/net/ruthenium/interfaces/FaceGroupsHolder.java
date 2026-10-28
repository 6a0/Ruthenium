package net.ruthenium.interfaces;

import net.minecraft.client.renderer.chunk.ChunkSectionLayer;
import net.ruthenium.render.chunk.SectionFaceGroups;
import org.jspecify.annotations.Nullable;

/**
 * Per-layer face groups of a section's compile results and of the compiled mesh made from them
 * ({@link SectionFaceGroups}).
 */
public interface FaceGroupsHolder {

    /** @return the layer's groups, or null if its quads were not grouped */
    @Nullable SectionFaceGroups ruthenium$getFaceGroups(ChunkSectionLayer layer);

    void ruthenium$setFaceGroups(ChunkSectionLayer layer, @Nullable SectionFaceGroups groups);
}
