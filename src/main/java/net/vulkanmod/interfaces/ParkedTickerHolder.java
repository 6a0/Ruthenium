package net.vulkanmod.interfaces;

/** Implemented by block entities, which remember their parked ticker ({@link ParkableTicker}) */
public interface ParkedTickerHolder {
    void vulkanmod$setParkedTicker(ParkableTicker ticker);

    /** Unparks this block entity's ticker, if parked. Called by every event that can end the idle state. */
    void vulkanmod$wakeTicker();
}
