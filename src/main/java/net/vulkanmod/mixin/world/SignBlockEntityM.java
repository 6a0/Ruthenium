package net.vulkanmod.mixin.world;

import net.minecraft.world.level.block.entity.HangingSignBlockEntity;
import net.minecraft.world.level.block.entity.SignBlockEntity;
import net.vulkanmod.interfaces.IdleTicker;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;

import java.util.UUID;

/** Vanilla's sign ticker only clears an editing player who walked away */
@Mixin(SignBlockEntity.class)
public abstract class SignBlockEntityM implements IdleTicker {
    @Shadow public abstract UUID getPlayerWhoMayEdit();

    @Override
    public boolean vulkanmod$isTickIdle() {
        Class<?> type = this.getClass();
        return (type == SignBlockEntity.class || type == HangingSignBlockEntity.class) && this.getPlayerWhoMayEdit() == null;
    }
}
