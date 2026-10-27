package net.vulkanmod.mixin.render.blockentity;

import net.minecraft.client.renderer.chunk.ChunkSectionLayer;
import net.minecraft.client.renderer.chunk.CompiledSectionMesh;
import net.minecraft.client.renderer.chunk.SectionCompiler;
import net.minecraft.client.renderer.chunk.TranslucencyPointOfView;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.vulkanmod.interfaces.BlockEntityPositions;
import net.vulkanmod.interfaces.FaceGroupsHolder;
import net.vulkanmod.render.blockentity.BlockEntityCulling;
import net.vulkanmod.render.chunk.SectionFaceGroups;
import org.jspecify.annotations.Nullable;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import java.util.List;

/**
 * The renderable block entity list of a compiled section never changes, so its positions are packed once (on the
 * render thread, when culling first needs them). The mesh also keeps the face groups of its compile results.
 */
@Mixin(CompiledSectionMesh.class)
public class CompiledSectionMeshM implements BlockEntityPositions, FaceGroupsHolder {
    @Shadow @Final private List<BlockEntity> renderableBlockEntities;

    @Unique private long[] positions;
    @Unique private boolean[] cullable;
    @Unique private final SectionFaceGroups[] faceGroups = new SectionFaceGroups[ChunkSectionLayer.values().length];

    @Inject(method = "<init>", at = @At("RETURN"))
    private void copyFaceGroups(TranslucencyPointOfView pointOfView, SectionCompiler.Results results, long compileTaskStartTimeNs,
                                CallbackInfo ci) {
        for (ChunkSectionLayer layer : ChunkSectionLayer.values()) {
            this.faceGroups[layer.ordinal()] = ((FaceGroupsHolder) (Object) results).vulkanmod$getFaceGroups(layer);
        }
    }

    @Override
    public @Nullable SectionFaceGroups vulkanmod$getFaceGroups(ChunkSectionLayer layer) {
        return this.faceGroups[layer.ordinal()];
    }

    @Override
    public void vulkanmod$setFaceGroups(ChunkSectionLayer layer, @Nullable SectionFaceGroups groups) {
        this.faceGroups[layer.ordinal()] = groups;
    }

    @Override
    public long[] vulkanmod$getPositions() {
        if (this.positions == null) {
            this.pack();
        }
        return this.positions;
    }

    @Override
    public boolean[] vulkanmod$getCullable() {
        if (this.cullable == null) {
            this.pack();
        }
        return this.cullable;
    }

    @Unique
    private void pack() {
        int size = this.renderableBlockEntities.size();
        long[] positions = new long[size];
        boolean[] cullable = new boolean[size];
        for (int i = 0; i < size; i++) {
            BlockEntity blockEntity = this.renderableBlockEntities.get(i);
            positions[i] = blockEntity.getBlockPos().asLong();
            cullable[i] = BlockEntityCulling.isCullable(blockEntity.getType());
        }
        this.cullable = cullable;
        this.positions = positions;
    }
}
