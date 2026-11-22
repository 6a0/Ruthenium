package dev.ruthenium.trace.mixin;

import net.minecraft.client.multiplayer.MultiPlayerGameMode;
import net.minecraft.core.BlockPos;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.gen.Accessor;

/** The block being broken, and how far along. */
@Mixin(MultiPlayerGameMode.class)
public interface MultiPlayerGameModeAccessor {
    @Accessor("destroyBlockPos")
    BlockPos ruthenium$destroyBlockPos();

    @Accessor("destroyProgress")
    float ruthenium$destroyProgress();

    @Accessor("destroyTicks")
    float ruthenium$destroyTicks();

    @Accessor("destroyDelay")
    int ruthenium$destroyDelay();
}
