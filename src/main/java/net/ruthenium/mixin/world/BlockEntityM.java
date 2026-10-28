package net.ruthenium.mixin.world;

import net.minecraft.world.level.block.entity.BlockEntity;
import net.ruthenium.interfaces.ParkableTicker;
import net.ruthenium.interfaces.ParkedTickerHolder;
import org.jspecify.annotations.Nullable;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;

@Mixin(BlockEntity.class)
public class BlockEntityM implements ParkedTickerHolder {
    @Unique private @Nullable ParkableTicker parkedTicker;

    @Override
    public void ruthenium$setParkedTicker(ParkableTicker ticker) {
        this.parkedTicker = ticker;
    }

    @Override
    public void ruthenium$wakeTicker() {
        ParkableTicker ticker = this.parkedTicker;
        if (ticker != null) {
            this.parkedTicker = null;
            ticker.ruthenium$unpark();
        }
    }
}
