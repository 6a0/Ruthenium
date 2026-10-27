package net.vulkanmod.interfaces;

import net.fabricmc.fabric.api.client.renderer.v1.mesh.Mesh;
import net.vulkanmod.render.entity.BlockModelMeshCache;
import org.jspecify.annotations.Nullable;

/** A block model render state that submits a mesh of {@link BlockModelMeshCache} instead of its own mesh's copy */
public interface CachedMeshRenderState {
    void vulkanmod$setCachedMesh(@Nullable Mesh mesh);
}
