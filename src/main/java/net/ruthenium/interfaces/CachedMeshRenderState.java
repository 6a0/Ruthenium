package net.ruthenium.interfaces;

import net.fabricmc.fabric.api.client.renderer.v1.mesh.Mesh;
import net.ruthenium.render.entity.BlockModelMeshCache;
import org.joml.Matrix4fc;
import org.jspecify.annotations.Nullable;

/** A block model render state that submits a mesh of {@link BlockModelMeshCache} instead of its own mesh's copy */
public interface CachedMeshRenderState {
    void ruthenium$setCachedMesh(@Nullable Mesh mesh);

    /**
     * Sets the state up as the Fabric Renderer API's {@code setupMesh} does, but to submit {@code mesh} and without a mesh
     * of its own
     *
     * @return false if the state may hold a mesh of its own (then {@code setupMesh} must run to clear it) and nothing was done
     */
    boolean ruthenium$setupCachedMesh(Matrix4fc transformation, boolean hasTranslucency, Mesh mesh);
}
