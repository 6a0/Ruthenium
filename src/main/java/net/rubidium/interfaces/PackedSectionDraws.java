package net.rubidium.interfaces;

import com.mojang.renderpearl.api.buffers.GpuBuffer;
import org.jspecify.annotations.Nullable;

/**
 * A compiled mesh's solid and cutout draws in the terrain uber buffers, packed on first use by
 * {@code TerrainDrawList} (see {@code TerrainDrawList.pack}).
 */
public interface PackedSectionDraws {

    int @Nullable [] rubidium$getPackedDraws();

    GpuBuffer @Nullable [] rubidium$getPackedBuffers();

    void rubidium$setPackedDraws(int[] draws, GpuBuffer[] buffers);
}
