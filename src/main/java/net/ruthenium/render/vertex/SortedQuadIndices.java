package net.ruthenium.render.vertex;

import com.mojang.renderpearl.api.pipeline.IndexType;
import org.lwjgl.system.MemoryUtil;

import java.nio.BufferOverflowException;
import java.nio.ByteBuffer;
import java.nio.ByteOrder;

/**
 * Writes the indices of sorted quads (vanilla's {@code MeshData.SortState} layout: 0 1 2 2 3 0 per quad, relative to the
 * draw's first vertex) straight to memory instead of through an {@code IntConsumer} per index.
 */
public final class SortedQuadIndices {
    private SortedQuadIndices() {
    }

    /** Writes the indices of the quads in {@code order} at the buffer's position and advances it */
    public static void write(ByteBuffer target, int[] order, IndexType indexType) {
        int size = order.length * 6 * indexType.bytes;

        if (target.isDirect() && target.order() == ByteOrder.LITTLE_ENDIAN && ByteOrder.nativeOrder() == ByteOrder.LITTLE_ENDIAN) {
            if (target.remaining() < size) {
                throw new BufferOverflowException();
            }

            write(order, MemoryUtil.memAddress(target), indexType);
            target.position(target.position() + size);
            return;
        }

        if (indexType == IndexType.SHORT) {
            for (int startIndex : order) {
                int base = startIndex * 4;
                target.putShort((short) base).putShort((short) (base + 1)).putShort((short) (base + 2))
                        .putShort((short) (base + 2)).putShort((short) (base + 3)).putShort((short) base);
            }
        } else {
            for (int startIndex : order) {
                int base = startIndex * 4;
                target.putInt(base).putInt(base + 1).putInt(base + 2)
                        .putInt(base + 2).putInt(base + 3).putInt(base);
            }
        }
    }

    /** Writes the indices of the quads in {@code order} at {@code pointer} */
    public static void write(int[] order, long pointer, IndexType indexType) {
        if (indexType == IndexType.SHORT) {
            for (int startIndex : order) {
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
            for (int startIndex : order) {
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
