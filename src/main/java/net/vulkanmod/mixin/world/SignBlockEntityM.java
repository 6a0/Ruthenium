package net.vulkanmod.mixin.world;

import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.entity.HangingSignBlockEntity;
import net.minecraft.world.level.block.entity.SignBlockEntity;
import net.minecraft.world.level.block.entity.SignText;
import net.minecraft.world.level.block.entity.SignTextSlot;
import net.vulkanmod.interfaces.IdleTicker;
import net.vulkanmod.render.blockentity.RetainedBlockEntities;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import java.util.UUID;

/**
 * Vanilla's sign ticker only clears an editing player who walked away. Text set on the client (the sign edit screen
 * does so while typing) rebuilds the sign's retained geometry.
 */
@Mixin(SignBlockEntity.class)
public abstract class SignBlockEntityM implements IdleTicker {
    @Shadow public abstract UUID getPlayerWhoMayEdit();

    @Inject(method = "setText", at = @At("RETURN"))
    private void markRetainedDirty(SignText text, SignTextSlot slot, CallbackInfo ci) {
        SignBlockEntity self = (SignBlockEntity) (Object) this;
        Level level = self.getLevel();
        if (level != null && level.isClientSide()) {
            RetainedBlockEntities.markDirty(self.getBlockPos());
        }
    }

    @Override
    public boolean vulkanmod$isTickIdle() {
        Class<?> type = this.getClass();
        return (type == SignBlockEntity.class || type == HangingSignBlockEntity.class) && this.getPlayerWhoMayEdit() == null;
    }
}
