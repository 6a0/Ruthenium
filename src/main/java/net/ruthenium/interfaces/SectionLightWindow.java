package net.ruthenium.interfaces;

import net.minecraft.core.BlockPos;
import net.ruthenium.render.chunk.SectionScratch;

/** {@code BlockModelLighter.Cache} with the light of a whole section kept while it compiles */
public interface SectionLightWindow {
    /** Uses the light window of {@code scratch} until the cache is disabled at the end of the compile */
    void ruthenium$beginSection(BlockPos sectionOrigin, SectionScratch scratch);
}
