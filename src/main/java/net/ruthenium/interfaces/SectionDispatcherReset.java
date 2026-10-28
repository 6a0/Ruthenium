package net.ruthenium.interfaces;

/** Implemented by {@code LevelRenderer} */
public interface SectionDispatcherReset {

    /**
     * Releases the compiled sections and the section dispatcher (with its uber buffers), keeping the level's chunk
     * tracking, so the next {@code LevelExtractor.allChanged} creates them again. Render thread only.
     */
    void ruthenium$recreateSectionDispatcher();
}
