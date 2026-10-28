package net.vulkanmod.interfaces;

/** A chunk section's count of block changes on the client (see {@code SupportingBlocks}) */
public interface SectionChangeStamp {
    int vulkanmod$changes();

    void vulkanmod$changed();
}
