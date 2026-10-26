package net.vulkanmod.mixin.world;

import net.minecraft.world.level.block.entity.BlockEntity;
import net.vulkanmod.interfaces.IdleTicker;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;

@Mixin(targets = "net.minecraft.world.level.chunk.LevelChunk$BoundTickingBlockEntity")
public class BoundTickingBlockEntityM implements IdleTicker {
    @Shadow @Final private BlockEntity blockEntity;

    @Override
    public boolean vulkanmod$isTickIdle() {
        return this.blockEntity instanceof IdleTicker idle && idle.vulkanmod$isTickIdle();
    }
}
