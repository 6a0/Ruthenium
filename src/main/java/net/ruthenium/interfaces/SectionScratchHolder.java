package net.ruthenium.interfaces;

import net.ruthenium.render.chunk.SectionScratch;

/** A {@code RenderSectionRegion} that uses the scratch of the compile it is in ({@code RenderSectionRegionM}) */
public interface SectionScratchHolder {
    /** Binds the region to {@code scratch} for a compile on the current thread */
    void ruthenium$bindScratch(SectionScratch scratch);

    /** Ends the binding: the scratch goes back to the pool */
    void ruthenium$unbindScratch();
}
