package net.vulkanmod.mixin.render.chunk;

import net.minecraft.client.renderer.chunk.ChunkSectionLayer;
import net.minecraft.client.renderer.chunk.SectionCompiler;
import net.vulkanmod.interfaces.FaceGroupsHolder;
import net.vulkanmod.render.chunk.SectionFaceGroups;
import org.jspecify.annotations.Nullable;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;

@Mixin(SectionCompiler.Results.class)
public class SectionCompilerResultsM implements FaceGroupsHolder {
    // Allocated in code rather than by an initializer (see CompiledSectionMeshM)
    @Unique private SectionFaceGroups @Nullable [] faceGroups;

    @Override
    public @Nullable SectionFaceGroups vulkanmod$getFaceGroups(ChunkSectionLayer layer) {
        return this.faceGroups != null ? this.faceGroups[layer.ordinal()] : null;
    }

    @Override
    public void vulkanmod$setFaceGroups(ChunkSectionLayer layer, @Nullable SectionFaceGroups groups) {
        if (this.faceGroups == null) {
            this.faceGroups = new SectionFaceGroups[ChunkSectionLayer.values().length];
        }
        this.faceGroups[layer.ordinal()] = groups;
    }
}
