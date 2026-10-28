package net.rubidium.mixin.world;

import net.minecraft.world.level.block.entity.ChestBlockEntity;
import net.minecraft.world.level.block.entity.ChestLidController;
import net.minecraft.world.level.block.entity.TrappedChestBlockEntity;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.rubidium.interfaces.IdleTicker;
import net.rubidium.interfaces.ParkedTickerHolder;
import net.rubidium.render.blockentity.RetainedBlockEntities;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

/** The client ticker of vanilla chests only animates the lid ({@code lidAnimateTick}) */
@Mixin(ChestBlockEntity.class)
public class ChestBlockEntityM implements IdleTicker {
    @Shadow @Final private ChestLidController chestLidController;

    @Override
    public boolean rubidium$isTickIdle() {
        Class<?> type = this.getClass();
        return (type == ChestBlockEntity.class || type == TrappedChestBlockEntity.class)
                && ((IdleTicker) this.chestLidController).rubidium$isTickIdle();
    }

    /**
     * Lid events start or stop the lid: the ticker is woken, and the chest leaves or rejoins its section's retained
     * geometry
     */
    @Inject(method = "triggerEvent", at = @At("RETURN"))
    private void markRetainedDirty(int b0, int b1, CallbackInfoReturnable<Boolean> cir) {
        BlockEntity self = (BlockEntity) (Object) this;
        ((ParkedTickerHolder) self).rubidium$wakeTicker();
        Level level = self.getLevel();
        if (level != null && level.isClientSide()) {
            RetainedBlockEntities.markDirty(self.getBlockPos());
        }
    }
}
