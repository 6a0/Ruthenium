package net.ruthenium.interfaces;

import net.minecraft.core.BlockPos;

/** {@code BlockModelLighter.Cache} with the light of a whole section kept while it compiles */
public interface SectionLightWindow {
    void ruthenium$beginSection(BlockPos sectionOrigin);
}
