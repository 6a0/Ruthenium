package net.ruthenium.mixin.render.vertex;

import com.mojang.blaze3d.vertex.ByteBufferBuilder;
import com.mojang.blaze3d.vertex.CompactVectorArray;
import com.mojang.blaze3d.vertex.MeshData;
import com.mojang.blaze3d.vertex.VertexSorting;
import com.mojang.renderpearl.api.pipeline.IndexType;
import net.ruthenium.render.vertex.SortedQuadIndices;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Overwrite;
import org.spongepowered.asm.mixin.Shadow;

import java.nio.ByteBuffer;

/**
 * Writes sorted quad indices straight to memory. Vanilla goes through an {@code IntConsumer} per index (plus a
 * {@code ByteBuffer.putInt} with bounds checks), which showed up at ~5% of the render thread with many signs in
 * view: sign text is sorted on upload, and enough of it needs 32-bit indices.
 */
@Mixin(MeshData.SortState.class)
public abstract class MeshDataSortStateM {
    @Shadow @Final private CompactVectorArray centroids;
    @Shadow @Final private IndexType indexType;

    /**
     * @author Ruthenium
     * @reason write indices directly
     */
    @Overwrite
    public ByteBufferBuilder.Result buildSortedIndexBuffer(ByteBufferBuilder target, VertexSorting sorting) {
        int[] startIndices = sorting.sort(this.centroids);
        long pointer = target.reserve(startIndices.length * 6 * this.indexType.bytes);
        SortedQuadIndices.write(startIndices, pointer, this.indexType);
        return target.build();
    }

    /**
     * @author Ruthenium
     * @reason write indices directly
     */
    @Overwrite
    public void writeSortedIndexBuffer(ByteBuffer target, VertexSorting sorting) {
        SortedQuadIndices.write(target, sorting.sort(this.centroids), this.indexType);
    }
}
