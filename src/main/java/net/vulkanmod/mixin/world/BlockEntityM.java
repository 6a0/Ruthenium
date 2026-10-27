package net.vulkanmod.mixin.world;

import net.minecraft.world.level.block.entity.BlockEntity;
import net.vulkanmod.interfaces.ParkableTicker;
import net.vulkanmod.interfaces.ParkedTickerHolder;
import org.jspecify.annotations.Nullable;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;

@Mixin(BlockEntity.class)
public class BlockEntityM implements ParkedTickerHolder {
    @Unique private @Nullable ParkableTicker parkedTicker;

    @Override
    public void vulkanmod$setParkedTicker(ParkableTicker ticker) {
        this.parkedTicker = ticker;
    }

    @Override
    public void vulkanmod$wakeTicker() {
        ParkableTicker ticker = this.parkedTicker;
        if (ticker != null) {
            this.parkedTicker = null;
            ticker.vulkanmod$unpark();
        }
    }
}
