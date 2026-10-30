package net.ruthenium.interfaces;

import net.minecraft.client.renderer.DynamicGpuData;
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

    /** The section info {@code TerrainDrawList} last wrote for the mesh's section, kept while its values stay the same */
    DynamicGpuData.@Nullable ChunkSectionInfo ruthenium$getSectionInfo();

    void ruthenium$setSectionInfo(DynamicGpuData.ChunkSectionInfo info);

    /** Whether {@code TerrainOcclusion} last saw the mesh's section visible: its solid and cutout layers are occluders */
    boolean ruthenium$isOccluder();

    void ruthenium$setOccluder(boolean occluder);
}
