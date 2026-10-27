package net.vulkanmod.mixin.world;

import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.entity.TickingBlockEntity;
import net.vulkanmod.interfaces.BlockEntityTicker;
import net.vulkanmod.interfaces.IdleTicker;
import net.vulkanmod.interfaces.ParkableTicker;
import net.vulkanmod.interfaces.ParkedTickerHolder;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/**
 * The wrapper the level's ticker list holds for each ticking block entity. Its ticker is rebound when the block
 * entity's state or the block entity itself changes, and to a null ticker when it is removed or its chunk unloads.
 */
@Mixin(targets = "net.minecraft.world.level.chunk.LevelChunk$RebindableTickingBlockEntityWrapper")
public class RebindableTickingBlockEntityWrapperM implements IdleTicker, ParkableTicker {
    @Shadow private TickingBlockEntity ticker;

    @Unique private boolean parked;
    @Unique private boolean listed;

    @Override
    public boolean vulkanmod$isTickIdle() {
        return this.ticker instanceof IdleTicker idle && idle.vulkanmod$isTickIdle();
    }

    @Override
    public boolean vulkanmod$tryPark() {
        if (this.ticker instanceof BlockEntityTicker bound && this.ticker instanceof IdleTicker idle && idle.vulkanmod$isTickIdle()) {
            ((ParkedTickerHolder) bound.vulkanmod$getBlockEntity()).vulkanmod$setParkedTicker(this);
            this.parked = true;
            return true;
        }
        return false;
    }

    @Override
    public boolean vulkanmod$isParked() {
        return this.parked;
    }

    @Override
    public void vulkanmod$unpark() {
        if (!this.parked) {
            return;
        }
        this.parked = false;
        // A null ticker (removed block entity, unloaded chunk) needs no list entry
        if (this.ticker instanceof BlockEntityTicker bound) {
            BlockEntity blockEntity = bound.vulkanmod$getBlockEntity();
            Level level = blockEntity.getLevel();
            if (level != null && !blockEntity.isRemoved()) {
                level.addBlockEntityTicker((TickingBlockEntity) (Object) this);
            }
        }
    }

    @Override
    public boolean vulkanmod$isListed() {
        return this.listed;
    }

    @Override
    public void vulkanmod$setListed(boolean listed) {
        this.listed = listed;
    }

    /** A new ticker (new block state or block entity) may not be idle */
    @Inject(method = "rebind", at = @At("RETURN"))
    private void unparkOnRebind(TickingBlockEntity ticker, CallbackInfo ci) {
        this.vulkanmod$unpark();
    }
}
