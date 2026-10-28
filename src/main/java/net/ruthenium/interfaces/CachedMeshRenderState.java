package net.ruthenium.interfaces;

import net.fabricmc.fabric.api.client.renderer.v1.mesh.Mesh;
import net.ruthenium.render.entity.BlockModelMeshCache;
import org.jspecify.annotations.Nullable;

/** A block model render state that submits a mesh of {@link BlockModelMeshCache} instead of its own mesh's copy */
public interface CachedMeshRenderState {
    void ruthenium$setCachedMesh(@Nullable Mesh mesh);
}
