package net.rubidium.interfaces;

/** Implemented by levels, which keep track of their parked tickers ({@link ParkableTicker}) */
public interface TickerParkingLevel {
    /** Puts every parked ticker back into the ticker list, e.g. when parking is switched off */
    void rubidium$unparkAll();
}
