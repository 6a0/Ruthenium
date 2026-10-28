package net.ruthenium.interfaces;

/** A chunk section's count of block changes on the client (see {@code SupportingBlocks}) */
public interface SectionChangeStamp {
    int ruthenium$changes();

    void ruthenium$changed();
}
