package dev.ruthenium.trace.mixin;

import net.minecraft.world.entity.LivingEntity;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.gen.Accessor;

/** A riptide spin's ticks left. */
@Mixin(LivingEntity.class)
public interface LivingEntityAccessor {
    @Accessor("autoSpinAttackTicks")
    int ruthenium$autoSpinAttackTicks();
}
