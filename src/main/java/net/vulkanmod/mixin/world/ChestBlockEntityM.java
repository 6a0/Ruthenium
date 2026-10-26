package net.vulkanmod.mixin.world;

import net.minecraft.world.level.block.entity.ChestBlockEntity;
import net.minecraft.world.level.block.entity.ChestLidController;
import net.minecraft.world.level.block.entity.TrappedChestBlockEntity;
import net.vulkanmod.interfaces.IdleTicker;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;

/** The client ticker of vanilla chests only animates the lid ({@code lidAnimateTick}) */
@Mixin(ChestBlockEntity.class)
public class ChestBlockEntityM implements IdleTicker {
    @Shadow @Final private ChestLidController chestLidController;

    @Override
    public boolean vulkanmod$isTickIdle() {
        Class<?> type = this.getClass();
        return (type == ChestBlockEntity.class || type == TrappedChestBlockEntity.class)
                && ((IdleTicker) this.chestLidController).vulkanmod$isTickIdle();
    }
}
