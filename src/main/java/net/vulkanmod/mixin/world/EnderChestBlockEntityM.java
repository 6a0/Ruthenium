package net.vulkanmod.mixin.world;

import net.minecraft.world.level.block.entity.ChestLidController;
import net.minecraft.world.level.block.entity.EnderChestBlockEntity;
import net.vulkanmod.interfaces.IdleTicker;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;

/** The client ticker of vanilla ender chests only animates the lid ({@code lidAnimateTick}) */
@Mixin(EnderChestBlockEntity.class)
public class EnderChestBlockEntityM implements IdleTicker {
    @Shadow @Final private ChestLidController chestLidController;

    @Override
    public boolean vulkanmod$isTickIdle() {
        Class<?> type = this.getClass();
        return type == EnderChestBlockEntity.class
                && ((IdleTicker) this.chestLidController).vulkanmod$isTickIdle();
    }
}
