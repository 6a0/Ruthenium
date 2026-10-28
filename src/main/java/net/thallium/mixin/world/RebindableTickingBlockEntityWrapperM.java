package net.thallium.mixin.world;

import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.EnchantingTableBlock;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.entity.EnchantingTableBlockEntity;
import net.minecraft.world.level.block.entity.TickingBlockEntity;
import net.minecraft.world.level.block.state.BlockState;
import net.thallium.interfaces.BlockEntityTicker;
import net.thallium.interfaces.IdleTicker;
import net.thallium.interfaces.ParkableTicker;
import net.thallium.interfaces.ParkedTickerHolder;
import net.thallium.world.EnchantingTableBatch;
import org.jspecify.annotations.Nullable;
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
    /** The enchanting table batch holding this parked ticker, and its slot there */
    @Unique private @Nullable EnchantingTableBatch batch;
    @Unique private int batchSlot;

    @Override
    public boolean thallium$isTickIdle() {
        return this.ticker instanceof IdleTicker idle && idle.thallium$isTickIdle();
    }

    @Override
    public boolean thallium$tryPark() {
        if (this.ticker instanceof BlockEntityTicker bound && this.ticker instanceof IdleTicker idle && idle.thallium$isTickIdle()) {
            ((ParkedTickerHolder) bound.thallium$getBlockEntity()).thallium$setParkedTicker(this);
            this.parked = true;
            return true;
        }
        return false;
    }

    @Override
    public boolean thallium$tryBatch(EnchantingTableBatch batch) {
        if (this.ticker instanceof BlockEntityTicker bound
                && bound.thallium$getBlockEntity() instanceof EnchantingTableBlockEntity table
                && table.getClass() == EnchantingTableBlockEntity.class) {
            // The block's class decides the ticker (EnchantingTableBlock.getTicker: bookAnimationTick on the client)
            BlockState state = bound.thallium$getValidState();
            if (state != null && state.getBlock().getClass() == EnchantingTableBlock.class) {
                this.parked = true;
                this.batch = batch;
                this.batchSlot = batch.add(this, table);
                return true;
            }
        }
        return false;
    }

    @Override
    public void thallium$setBatchSlot(int slot) {
        this.batchSlot = slot;
    }

    @Override
    public void thallium$leaveBatch() {
        this.parked = false;
        this.batch = null;
    }

    @Override
    public @Nullable BlockState thallium$tickedState() {
        return this.ticker instanceof BlockEntityTicker bound ? bound.thallium$getValidState() : null;
    }

    @Override
    public boolean thallium$isParked() {
        return this.parked;
    }

    @Override
    public void thallium$unpark() {
        if (!this.parked) {
            return;
        }
        this.parked = false;
        if (this.batch != null) {
            this.batch.remove(this.batchSlot);
            this.batch = null;
        }
        // A null ticker (removed block entity, unloaded chunk) needs no list entry
        if (this.ticker instanceof BlockEntityTicker bound) {
            BlockEntity blockEntity = bound.thallium$getBlockEntity();
            Level level = blockEntity.getLevel();
            if (level != null && !blockEntity.isRemoved()) {
                level.addBlockEntityTicker((TickingBlockEntity) (Object) this);
            }
        }
    }

    @Override
    public boolean thallium$isListed() {
        return this.listed;
    }

    @Override
    public void thallium$setListed(boolean listed) {
        this.listed = listed;
    }

    /** A new ticker (new block state or block entity) may not be idle */
    @Inject(method = "rebind", at = @At("RETURN"))
    private void unparkOnRebind(TickingBlockEntity ticker, CallbackInfo ci) {
        this.thallium$unpark();
    }
}
