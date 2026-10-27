package net.vulkanmod.interfaces;

import net.minecraft.world.level.block.entity.BlockEntity;

/** Implemented by the ticker bound to a block entity */
public interface BlockEntityTicker {
    BlockEntity vulkanmod$getBlockEntity();
}
