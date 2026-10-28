package net.thallium.interfaces;

import net.minecraft.world.level.block.state.BlockState;
import net.thallium.world.EnchantingTableBatch;
import org.jspecify.annotations.Nullable;

/**
 * Implemented by the level's block entity ticker wrappers. An idle ticker ({@link IdleTicker}) can be parked: it
 * leaves the level's ticker list until its block entity wakes it ({@link ParkedTickerHolder}) or the wrapper is
 * rebound to another ticker, so the client doesn't visit it every tick only to skip it.
 */
public interface ParkableTicker {
    /** Parks this ticker if its tick would do nothing; the caller then drops it from the level's list */
    boolean thallium$tryPark();

    boolean thallium$isParked();

    /** Puts a parked ticker back into its level's ticker list */
    void thallium$unpark();

    /** Whether the level's list of parked tickers holds this ticker (it can stay there after a wake) */
    boolean thallium$isListed();

    void thallium$setListed(boolean listed);

    /**
     * Parks this ticker into the enchanting table batch if it ticks a vanilla enchanting table with vanilla's ticker
     * and a validated block state ({@link EnchantingTableBatch})
     *
     * @return false if this ticker can't be batched
     */
    boolean thallium$tryBatch(EnchantingTableBatch batch);

    /** The ticker's slot in its batch moved */
    void thallium$setBatchSlot(int slot);

    /** Leaves the batch without returning to the ticker list (the block entity was removed) */
    void thallium$leaveBatch();

    /** The validated block state of the bound ticker, or null */
    @Nullable BlockState thallium$tickedState();
}
