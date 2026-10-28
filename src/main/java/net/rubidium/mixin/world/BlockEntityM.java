package net.rubidium.mixin.world;

import net.minecraft.world.level.block.entity.BlockEntity;
import net.rubidium.interfaces.ParkableTicker;
import net.rubidium.interfaces.ParkedTickerHolder;
import org.jspecify.annotations.Nullable;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;

@Mixin(BlockEntity.class)
public class BlockEntityM implements ParkedTickerHolder {
    @Unique private @Nullable ParkableTicker parkedTicker;

    @Override
    public void rubidium$setParkedTicker(ParkableTicker ticker) {
        this.parkedTicker = ticker;
    }

    @Override
    public void rubidium$wakeTicker() {
        ParkableTicker ticker = this.parkedTicker;
        if (ticker != null) {
            this.parkedTicker = null;
            ticker.rubidium$unpark();
        }
    }
}
