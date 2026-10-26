package net.vulkanmod.mixin.render.vertex;

import com.mojang.blaze3d.vertex.ByteBufferBuilder;
import com.mojang.blaze3d.vertex.CompactVectorArray;
import com.mojang.blaze3d.vertex.MeshData;
import com.mojang.blaze3d.vertex.VertexSorting;
import com.mojang.renderpearl.api.pipeline.IndexType;
import org.lwjgl.system.MemoryUtil;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Overwrite;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.Unique;

import java.nio.BufferOverflowException;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;

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
     * @author VulkanMod
     * @reason write indices directly
     */
    @Overwrite
    public ByteBufferBuilder.Result buildSortedIndexBuffer(ByteBufferBuilder target, VertexSorting sorting) {
        int[] startIndices = sorting.sort(this.centroids);
        long pointer = target.reserve(startIndices.length * 6 * this.indexType.bytes);
        writeIndices(startIndices, pointer, this.indexType);
        return target.build();
    }

    /**
     * @author VulkanMod
     * @reason write indices directly
     */
    @Overwrite
    public void writeSortedIndexBuffer(ByteBuffer target, VertexSorting sorting) {
        int[] startIndices = sorting.sort(this.centroids);
        int size = startIndices.length * 6 * this.indexType.bytes;

        if (target.isDirect() && target.order() == ByteOrder.LITTLE_ENDIAN && ByteOrder.nativeOrder() == ByteOrder.LITTLE_ENDIAN) {
            if (target.remaining() < size) {
                throw new BufferOverflowException();
            }

            writeIndices(startIndices, MemoryUtil.memAddress(target), this.indexType);
            target.position(target.position() + size);
            return;
        }

        if (this.indexType == IndexType.SHORT) {
            for (int startIndex : startIndices) {
                int base = startIndex * 4;
                target.putShort((short) base).putShort((short) (base + 1)).putShort((short) (base + 2))
                        .putShort((short) (base + 2)).putShort((short) (base + 3)).putShort((short) base);
            }
        } else {
            for (int startIndex : startIndices) {
                int base = startIndex * 4;
                target.putInt(base).putInt(base + 1).putInt(base + 2)
                        .putInt(base + 2).putInt(base + 3).putInt(base);
            }
        }
    }

    /** Index order per quad matches vanilla: 0 1 2 2 3 0. */
    @Unique
    private static void writeIndices(int[] startIndices, long pointer, IndexType indexType) {
        if (indexType == IndexType.SHORT) {
            for (int startIndex : startIndices) {
                int base = startIndex * 4;
                MemoryUtil.memPutShort(pointer, (short) base);
                MemoryUtil.memPutShort(pointer + 2, (short) (base + 1));
                MemoryUtil.memPutShort(pointer + 4, (short) (base + 2));
                MemoryUtil.memPutShort(pointer + 6, (short) (base + 2));
                MemoryUtil.memPutShort(pointer + 8, (short) (base + 3));
                MemoryUtil.memPutShort(pointer + 10, (short) base);
                pointer += 12;
            }
        } else {
            for (int startIndex : startIndices) {
                int base = startIndex * 4;
                MemoryUtil.memPutInt(pointer, base);
                MemoryUtil.memPutInt(pointer + 4, base + 1);
                MemoryUtil.memPutInt(pointer + 8, base + 2);
                MemoryUtil.memPutInt(pointer + 12, base + 2);
                MemoryUtil.memPutInt(pointer + 16, base + 3);
                MemoryUtil.memPutInt(pointer + 20, base);
                pointer += 24;
            }
        }
    }
}
