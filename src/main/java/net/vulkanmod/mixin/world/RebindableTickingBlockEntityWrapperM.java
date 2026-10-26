package net.vulkanmod.mixin.world;

import net.minecraft.world.level.block.entity.TickingBlockEntity;
import net.vulkanmod.interfaces.IdleTicker;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;

@Mixin(targets = "net.minecraft.world.level.chunk.LevelChunk$RebindableTickingBlockEntityWrapper")
public class RebindableTickingBlockEntityWrapperM implements IdleTicker {
    @Shadow private TickingBlockEntity ticker;

    @Override
    public boolean vulkanmod$isTickIdle() {
        return this.ticker instanceof IdleTicker idle && idle.vulkanmod$isTickIdle();
    }
}
