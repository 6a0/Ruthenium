package net.vulkanmod.gametest.mixin;

import net.minecraft.util.profiling.Profiler;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.entity.TickingBlockEntity;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import java.util.List;

/** Counts the block entity tickers the level visits per tick, for the light profiler */
@Mixin(value = Level.class, priority = 900)
public class LevelTickProfileMixin {
    @Shadow @Final protected List<TickingBlockEntity> blockEntityTickers;

    @Inject(method = "tickBlockEntities", at = @At("HEAD"))
    private void countTickers(CallbackInfo ci) {
        Profiler.get().incrementCounter("tickers", this.blockEntityTickers.size());
    }
}
