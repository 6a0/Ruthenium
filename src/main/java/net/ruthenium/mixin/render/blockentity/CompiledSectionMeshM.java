package net.ruthenium.mixin.render.blockentity;

import com.mojang.renderpearl.api.buffers.GpuBuffer;
import net.minecraft.client.renderer.chunk.ChunkSectionLayer;
import net.minecraft.client.renderer.chunk.CompiledSectionMesh;
import net.minecraft.client.renderer.chunk.SectionCompiler;
import net.minecraft.client.renderer.chunk.TranslucencyPointOfView;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.ruthenium.interfaces.BlockEntityPositions;
import net.ruthenium.interfaces.PackedSectionDraws;
import net.ruthenium.interfaces.FaceGroupsHolder;
import net.ruthenium.render.blockentity.BlockEntityCulling;
import net.ruthenium.render.chunk.SectionFaceGroups;
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
 * render thread, when culling first needs them). The mesh also keeps the face groups of its compile results and
 * its draws in the terrain buffers, packed by {@code TerrainDrawList}.
 */
@Mixin(CompiledSectionMesh.class)
public class CompiledSectionMeshM implements BlockEntityPositions, FaceGroupsHolder, PackedSectionDraws {
    @Shadow @Final private List<BlockEntity> renderableBlockEntities;

    @Unique private long[] positions;
    @Unique private boolean[] cullable;
    // Allocated in code: initializers of these fields were not merged into the constructor before the injection ran
    @Unique private SectionFaceGroups @Nullable [] faceGroups;
    @Unique private int @Nullable [] packedDraws;
    @Unique private GpuBuffer @Nullable [] packedBuffers;

    @Inject(method = "<init>", at = @At("RETURN"))
    private void copyFaceGroups(TranslucencyPointOfView pointOfView, SectionCompiler.Results results, long compileTaskStartTimeNs,
                                CallbackInfo ci) {
        for (ChunkSectionLayer layer : ChunkSectionLayer.values()) {
            SectionFaceGroups groups = ((FaceGroupsHolder) (Object) results).ruthenium$getFaceGroups(layer);
            if (groups != null) {
                this.ruthenium$setFaceGroups(layer, groups);
            }
        }
    }

    @Override
    public @Nullable SectionFaceGroups ruthenium$getFaceGroups(ChunkSectionLayer layer) {
        return this.faceGroups != null ? this.faceGroups[layer.ordinal()] : null;
    }

    @Override
    public void ruthenium$setFaceGroups(ChunkSectionLayer layer, @Nullable SectionFaceGroups groups) {
        if (this.faceGroups == null) {
            this.faceGroups = new SectionFaceGroups[ChunkSectionLayer.values().length];
        }
        this.faceGroups[layer.ordinal()] = groups;
    }

    @Override
    public int @Nullable [] ruthenium$getPackedDraws() {
        return this.packedDraws;
    }

    @Override
    public GpuBuffer @Nullable [] ruthenium$getPackedBuffers() {
        return this.packedBuffers;
    }

    @Override
    public void ruthenium$setPackedDraws(int[] draws, GpuBuffer[] buffers) {
        this.packedBuffers = buffers;
        this.packedDraws = draws;
    }

    @Override
    public long[] ruthenium$getPositions() {
        if (this.positions == null) {
            this.pack();
        }
        return this.positions;
    }

    @Override
    public boolean[] ruthenium$getCullable() {
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
