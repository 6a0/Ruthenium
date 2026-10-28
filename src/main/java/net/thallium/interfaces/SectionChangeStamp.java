package net.thallium.interfaces;

/** A chunk section's count of block changes on the client (see {@code SupportingBlocks}) */
public interface SectionChangeStamp {
    int thallium$changes();

    void thallium$changed();
}
