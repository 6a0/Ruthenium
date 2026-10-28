package net.vulkanmod.interfaces;

import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import org.jspecify.annotations.Nullable;

/** Implemented by the ticker bound to a block entity */
public interface BlockEntityTicker {
    BlockEntity vulkanmod$getBlockEntity();

    /** The block state that passed the validity check of a tick on the client, kept for the ticker's lifetime, or null */
    @Nullable BlockState vulkanmod$getValidState();
}
