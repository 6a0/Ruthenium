package net.vulkanmod.mixin.world;

import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.entity.TickingBlockEntity;
import net.vulkanmod.interfaces.IdleTicker;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Redirect;

/**
 * Skips block entity ticks that would do nothing (closed chests, signs nobody is editing), see {@link IdleTicker}.
 * A skipped tick also skips the chunk's world border check, block state lookup and validity check that guard it,
 * which were most of the cost: the client ticks every loaded chest every tick (~1.6 ms per tick with ~14k block
 * entities loaded).
 */
@Mixin(Level.class)
public class LevelM {
    @Redirect(method = "tickBlockEntities", at = @At(value = "INVOKE",
            target = "Lnet/minecraft/world/level/block/entity/TickingBlockEntity;tick()V"))
    private void skipIdleTicks(TickingBlockEntity ticker) {
        if (!(ticker instanceof IdleTicker idle && idle.vulkanmod$isTickIdle())) {
            ticker.tick();
        }
    }
}
