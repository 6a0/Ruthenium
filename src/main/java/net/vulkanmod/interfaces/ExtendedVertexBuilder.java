package net.vulkanmod.interfaces;

import com.mojang.blaze3d.vertex.VertexConsumer;
import com.mojang.renderpearl.api.vertex.VertexFormat;
import org.jetbrains.annotations.Nullable;

/**
 * Direct vertex writing into a {@link com.mojang.blaze3d.vertex.BufferBuilder}, bypassing the per-element
 * {@link VertexConsumer} calls for hot paths that emit whole quads.
 */
public interface ExtendedVertexBuilder {

    static @Nullable ExtendedVertexBuilder of(VertexConsumer vertexConsumer, VertexFormat format) {
        if (vertexConsumer instanceof ExtendedVertexBuilder builder && builder.canWriteDirect(format)) {
            return builder;
        }

        return null;
    }

    /**
     * @return true if vertices can be written directly in the given format (same format, non-line topology,
     * little-endian host)
     */
    boolean canWriteDirect(VertexFormat format);

    /**
     * Reserves {@code count} consecutive complete vertices; the caller must write all of them.
     *
     * @return the address of the first vertex
     */
    long reserveVertices(int count);
}
