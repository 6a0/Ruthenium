package net.ruthenium.mixin.world;

import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.entity.DecoratedPotBlockEntity;
import net.ruthenium.render.blockentity.RetainedBlockEntities;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

@Mixin(DecoratedPotBlockEntity.class)
public class DecoratedPotBlockEntityM {
    /** Wobble events start a wobble; the pot leaves its section's retained geometry until it ends */
    @Inject(method = "triggerEvent", at = @At("RETURN"))
    private void markRetainedDirty(int event, int data, CallbackInfoReturnable<Boolean> cir) {
        BlockEntity self = (BlockEntity) (Object) this;
        Level level = self.getLevel();
        if (level != null && level.isClientSide()) {
            RetainedBlockEntities.markDirty(self.getBlockPos());
        }
    }
}
