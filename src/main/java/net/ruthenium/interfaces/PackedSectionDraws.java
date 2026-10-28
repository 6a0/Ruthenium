package net.ruthenium.interfaces;

import com.mojang.renderpearl.api.buffers.GpuBuffer;
import org.jspecify.annotations.Nullable;

/**
 * A compiled mesh's solid and cutout draws in the terrain uber buffers, packed on first use by
 * {@code TerrainDrawList} (see {@code TerrainDrawList.pack}).
 */
public interface PackedSectionDraws {

    int @Nullable [] ruthenium$getPackedDraws();

    GpuBuffer @Nullable [] ruthenium$getPackedBuffers();

    void ruthenium$setPackedDraws(int[] draws, GpuBuffer[] buffers);
}
