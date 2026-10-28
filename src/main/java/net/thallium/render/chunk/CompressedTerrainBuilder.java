package net.thallium.render.chunk;

import com.mojang.blaze3d.vertex.BufferBuilder;
import com.mojang.blaze3d.vertex.ByteBufferBuilder;
import com.mojang.blaze3d.vertex.VertexConsumer;
import com.mojang.renderpearl.api.pipeline.PrimitiveTopology;
import net.minecraft.util.ARGB;
import net.thallium.interfaces.ExtendedVertexBuilder;
import org.lwjgl.system.MemoryUtil;

/**
 * The section compiler's builder for {@link CompressedTerrain#FORMAT}, so vanilla's {@code BufferBuilder} keeps
 * its own paths. Block models (vanilla's and Fabric's renderer), fluids and the per-element calls all encode the
 * position and UV0; color and light are stored as vanilla does. Little-endian hosts only (see
 * {@link CompressedTerrain#onDispatcherCreated}).
 */
public class CompressedTerrainBuilder extends BufferBuilder {
    private static final int UV0_ELEMENT = 2;

    public CompressedTerrainBuilder(ByteBufferBuilder buffer, PrimitiveTopology primitiveTopology) {
        super(buffer, primitiveTopology, CompressedTerrain.FORMAT);
    }

    @Override
    public void addVertex(float x, float y, float z, int color, float u, float v, int overlayCoords, int lightCoords,
                          float nx, float ny, float nz) {
        long pointer = ((ExtendedVertexBuilder) this).reserveVertices(1);
        MemoryUtil.memPutLong(pointer + CompressedTerrain.POSITION_OFFSET, CompressedTerrain.packPosition(x, y, z));
        MemoryUtil.memPutInt(pointer + CompressedTerrain.COLOR_OFFSET, ARGB.toABGR(color));
        MemoryUtil.memPutInt(pointer + CompressedTerrain.UV0_OFFSET, CompressedTerrain.packUv(u, v));
        MemoryUtil.memPutInt(pointer + CompressedTerrain.UV2_OFFSET, lightCoords);
    }

    @Override
    public VertexConsumer addVertex(float x, float y, float z) {
        // Begins the vertex (writing floats over the position and color, which the caller still has to set)
        super.addVertex(x, y, z);
        long pointer = ((ExtendedVertexBuilder) this).thallium$vertexPointer();
        MemoryUtil.memPutLong(pointer + CompressedTerrain.POSITION_OFFSET, CompressedTerrain.packPosition(x, y, z));
        return this;
    }

    @Override
    public VertexConsumer setUv(float u, float v) {
        long pointer = ((ExtendedVertexBuilder) this).thallium$beginElement(UV0_ELEMENT);
        if (pointer != -1L) {
            MemoryUtil.memPutInt(pointer, CompressedTerrain.packUv(u, v));
        }
        return this;
    }
}
