package net.vulkanmod.mixin.render.vertex;

import com.mojang.blaze3d.vertex.BufferBuilder;
import com.mojang.blaze3d.vertex.ByteBufferBuilder;
import com.mojang.renderpearl.api.pipeline.PrimitiveTopology;
import com.mojang.renderpearl.api.vertex.VertexFormat;
import net.vulkanmod.interfaces.ExtendedVertexBuilder;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;

import java.nio.ByteOrder;

@Mixin(BufferBuilder.class)
public abstract class BufferBuilderM implements ExtendedVertexBuilder {
    private static final boolean IS_LITTLE_ENDIAN = ByteOrder.nativeOrder() == ByteOrder.LITTLE_ENDIAN;
    private static final int MAX_VERTICES = 16777215;

    @Shadow @Final private ByteBufferBuilder buffer;
    @Shadow private long vertexPointer;
    @Shadow private int vertices;
    @Shadow @Final private VertexFormat format;
    @Shadow @Final private PrimitiveTopology primitiveTopology;
    @Shadow @Final private int vertexSize;

    @Shadow protected abstract void ensureBuilding();
    @Shadow protected abstract void endLastVertex();
    @Shadow protected abstract long beginElement(int semanticID);

    @Override
    public boolean canWriteDirect(VertexFormat format) {
        return this.format == format && this.primitiveTopology != PrimitiveTopology.LINES && IS_LITTLE_ENDIAN;
    }

    @Override
    public long reserveVertices(int count) {
        this.ensureBuilding();
        this.endLastVertex();

        if (this.vertices + count > MAX_VERTICES) {
            throw new IllegalStateException("Trying to write too many vertices (>16777215) into BufferBuilder");
        }

        long pointer = this.buffer.reserve(this.vertexSize * count);
        this.vertices += count;
        this.vertexPointer = pointer + (long) this.vertexSize * (count - 1);
        return pointer;
    }

    @Override
    public long vulkanmod$vertexPointer() {
        return this.vertexPointer;
    }

    @Override
    public long vulkanmod$beginElement(int semanticId) {
        return this.beginElement(semanticId);
    }
}
