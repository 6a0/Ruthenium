package net.ruthenium.mixin.world;

import net.minecraft.core.BlockPos;
import net.minecraft.world.TickRateManager;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.entity.TickingBlockEntity;
import net.ruthenium.Initializer;
import net.ruthenium.interfaces.IdleTicker;
import net.ruthenium.interfaces.ParkableTicker;
import net.ruthenium.interfaces.TickerParkingLevel;
import net.ruthenium.world.EnchantingTableBatch;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.Redirect;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import java.util.ArrayList;
import java.util.List;

/**
 * Skips block entity ticks that would do nothing (closed chests, signs nobody is editing), see {@link IdleTicker}.
 * A skipped tick also skips the chunk's world border check, block state lookup and validity check that guard it,
 * which were most of the cost: the client ticks every loaded chest every tick (~1.6 ms per tick with ~14k block
 * entities loaded).
 * <p>
 * On the client, idle tickers are also parked ({@link ParkableTicker}): they leave the ticker list until an event
 * that can end the idle state wakes them, because visiting ~10k idle tickers per tick only to skip them still took
 * ~0.75 ms (a few dependent loads each: wrapper, ticker, block entity, lid controller).
 */
@Mixin(Level.class)
public abstract class LevelM implements TickerParkingLevel {
    @Shadow @Final protected List<TickingBlockEntity> blockEntityTickers;
    @Shadow @Final private List<TickingBlockEntity> pendingBlockEntityTickers;
    @Shadow private boolean tickingBlockEntities;

    @Shadow public abstract boolean isClientSide();
    @Shadow public abstract TickRateManager tickRateManager();
    @Shadow public abstract boolean shouldTickBlocksAt(BlockPos pos);

    /** Parked tickers, for {@link #ruthenium$unparkAll}; may hold tickers woken since (each ticker at most once) */
    @Unique private List<ParkableTicker> parkedTickers;
    @Unique private int parkedCompactSize;
    /** The client's enchanting table tickers, ticked in one loop after the list ({@link EnchantingTableBatch}) */
    @Unique private EnchantingTableBatch tableBatch;

    @Redirect(method = "tickBlockEntities", at = @At(value = "INVOKE",
            target = "Lnet/minecraft/world/level/block/entity/TickingBlockEntity;tick()V"))
    private void skipIdleTicks(TickingBlockEntity ticker) {
        if (!(Initializer.CONFIG.skipIdleTicks && ticker instanceof IdleTicker idle && idle.ruthenium$isTickIdle())) {
            ticker.tick();
        }
    }

    /**
     * Vanilla's loop with idle tickers parked, compacting the list in place (vanilla removes tickers of removed
     * block entities through the iterator, which shifts the rest of the list each time)
     */
    @Inject(method = "tickBlockEntities", at = @At("HEAD"), cancellable = true)
    private void tickParkingIdle(CallbackInfo ci) {
        if (!this.isClientSide() || !Initializer.CONFIG.skipIdleTicks || !Initializer.CONFIG.parkIdleTickers) {
            return;
        }
        ci.cancel();

        this.tickingBlockEntities = true;
        if (!this.pendingBlockEntityTickers.isEmpty()) {
            this.blockEntityTickers.addAll(this.pendingBlockEntityTickers);
            this.pendingBlockEntityTickers.clear();
        }

        EnchantingTableBatch batch = this.tableBatch;
        boolean batchTables = Initializer.CONFIG.batchTableTicks && Initializer.CONFIG.cacheTickerStates;
        if (batchTables) {
            if (batch == null) {
                batch = this.tableBatch = new EnchantingTableBatch();
            }
        } else if (batch != null && batch.size() > 0) {
            batch.unparkAll();
        }

        List<TickingBlockEntity> tickers = this.blockEntityTickers;
        boolean tick = this.tickRateManager().runsNormally();
        int size = tickers.size();
        int kept = 0;
        int i = 0;
        try {
            for (; i < size; i++) {
                TickingBlockEntity ticker = tickers.get(i);
                if (ticker.isRemoved()) {
                    continue;
                }
                if (tick && this.shouldTickBlocksAt(ticker.getPos())) {
                    if (ticker instanceof ParkableTicker parkable) {
                        if (parkable.ruthenium$tryPark()) {
                            this.addParked(parkable);
                            continue;
                        }
                        if (batchTables && parkable.ruthenium$tryBatch(batch)) {
                            // Ticks with the batch below, this tick too
                            continue;
                        }
                    }
                    ticker.tick();
                }
                tickers.set(kept++, ticker);
            }
            if (batchTables) {
                batch.tick((Level) (Object) this, tick);
            }
        } finally {
            // Only after a tick threw: keep the tickers not visited yet
            for (; i < size; i++) {
                tickers.set(kept++, tickers.get(i));
            }
            if (kept < size) {
                tickers.subList(kept, size).clear();
            }
            this.tickingBlockEntities = false;
        }
    }

    @Unique
    private void addParked(ParkableTicker ticker) {
        if (this.parkedTickers == null) {
            this.parkedTickers = new ArrayList<>();
        }
        if (ticker.ruthenium$isListed()) {
            return;
        }
        List<ParkableTicker> parked = this.parkedTickers;
        if (parked.size() >= this.parkedCompactSize) {
            // Drop tickers woken since they were parked (amortized: the list at least doubles between compactions)
            parked.removeIf(t -> {
                if (t.ruthenium$isParked()) {
                    return false;
                }
                t.ruthenium$setListed(false);
                return true;
            });
            this.parkedCompactSize = Math.max(1024, parked.size() * 2);
        }
        ticker.ruthenium$setListed(true);
        parked.add(ticker);
    }

    @Override
    public void ruthenium$unparkAll() {
        if (this.tableBatch != null) {
            this.tableBatch.unparkAll();
        }
        List<ParkableTicker> parked = this.parkedTickers;
        if (parked == null) {
            return;
        }
        this.parkedTickers = null;
        this.parkedCompactSize = 0;
        for (ParkableTicker ticker : parked) {
            ticker.ruthenium$setListed(false);
            ticker.ruthenium$unpark();
        }
    }
}
