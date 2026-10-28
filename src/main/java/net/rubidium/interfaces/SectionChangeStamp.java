package net.rubidium.interfaces;

/** A chunk section's count of block changes on the client (see {@code SupportingBlocks}) */
public interface SectionChangeStamp {
    int rubidium$changes();

    void rubidium$changed();
}
