package net.rubidium.mixin.world;

import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.entity.ShulkerBoxBlockEntity;
import net.rubidium.interfaces.IdleTicker;
import net.rubidium.interfaces.ParkedTickerHolder;
import net.rubidium.render.blockentity.RetainedBlockEntities;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

/**
 * The ticker of vanilla shulker boxes only animates the lid ({@code updateAnimation}), which sets
 * {@code progressOld = progress = 0} while closed
 */
@Mixin(ShulkerBoxBlockEntity.class)
public class ShulkerBoxBlockEntityM implements IdleTicker {
    @Shadow private ShulkerBoxBlockEntity.AnimationStatus animationStatus;
    @Shadow private float progress;
    @Shadow private float progressOld;

    @Override
    public boolean rubidium$isTickIdle() {
        Class<?> type = this.getClass();
        return type == ShulkerBoxBlockEntity.class && this.animationStatus == ShulkerBoxBlockEntity.AnimationStatus.CLOSED
                && Float.floatToRawIntBits(this.progress) == 0 && Float.floatToRawIntBits(this.progressOld) == 0;
    }

    /** Lid events start the lid: the ticker is woken, and the box leaves its section's retained geometry */
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
