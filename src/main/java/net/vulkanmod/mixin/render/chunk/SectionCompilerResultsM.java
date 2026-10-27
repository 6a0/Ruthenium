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
    @Unique private final SectionFaceGroups[] faceGroups = new SectionFaceGroups[ChunkSectionLayer.values().length];

    @Override
    public @Nullable SectionFaceGroups vulkanmod$getFaceGroups(ChunkSectionLayer layer) {
        return this.faceGroups[layer.ordinal()];
    }

    @Override
    public void vulkanmod$setFaceGroups(ChunkSectionLayer layer, @Nullable SectionFaceGroups groups) {
        this.faceGroups[layer.ordinal()] = groups;
    }
}
