package net.vulkanmod.interfaces;

/**
 * Implemented by the level's block entity ticker wrappers. An idle ticker ({@link IdleTicker}) can be parked: it
 * leaves the level's ticker list until its block entity wakes it ({@link ParkedTickerHolder}) or the wrapper is
 * rebound to another ticker, so the client doesn't visit it every tick only to skip it.
 */
public interface ParkableTicker {
    /** Parks this ticker if its tick would do nothing; the caller then drops it from the level's list */
    boolean vulkanmod$tryPark();

    boolean vulkanmod$isParked();

    /** Puts a parked ticker back into its level's ticker list */
    void vulkanmod$unpark();

    /** Whether the level's list of parked tickers holds this ticker (it can stay there after a wake) */
    boolean vulkanmod$isListed();

    void vulkanmod$setListed(boolean listed);
}
