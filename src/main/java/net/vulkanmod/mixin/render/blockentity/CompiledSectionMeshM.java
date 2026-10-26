package net.vulkanmod.mixin.render.blockentity;

import net.minecraft.client.renderer.chunk.CompiledSectionMesh;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.vulkanmod.interfaces.BlockEntityPositions;
import net.vulkanmod.render.blockentity.BlockEntityCulling;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.Unique;

import java.util.List;

/**
 * The renderable block entity list of a compiled section never changes, so its positions are packed once (on the
 * render thread, when culling first needs them).
 */
@Mixin(CompiledSectionMesh.class)
public class CompiledSectionMeshM implements BlockEntityPositions {
    @Shadow @Final private List<BlockEntity> renderableBlockEntities;

    @Unique private long[] positions;
    @Unique private boolean[] cullable;

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
