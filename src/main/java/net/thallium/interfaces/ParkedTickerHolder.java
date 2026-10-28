package net.thallium.interfaces;

/** Implemented by block entities, which remember their parked ticker ({@link ParkableTicker}) */
public interface ParkedTickerHolder {
    void thallium$setParkedTicker(ParkableTicker ticker);

    /** Unparks this block entity's ticker, if parked. Called by every event that can end the idle state. */
    void thallium$wakeTicker();
}
