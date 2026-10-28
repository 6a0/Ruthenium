package net.ruthenium.mixin.render.vertex;

import com.mojang.blaze3d.vertex.BufferBuilder;
import com.mojang.blaze3d.vertex.ByteBufferBuilder;
import com.mojang.blaze3d.vertex.PoseStack;
import com.mojang.blaze3d.vertex.QuadInstance;
import com.mojang.blaze3d.vertex.VertexConsumer;
import com.mojang.renderpearl.api.pipeline.PrimitiveTopology;
import com.mojang.renderpearl.api.vertex.VertexFormat;
import net.minecraft.client.model.geom.builders.UVPair;
import net.minecraft.client.resources.model.geometry.BakedQuad;
import net.minecraft.util.ARGB;
import net.minecraft.util.Mth;
import net.ruthenium.Initializer;
import net.ruthenium.interfaces.ExtendedVertexBuilder;
import org.joml.Matrix4f;
import org.joml.Vector3f;
import org.lwjgl.system.MemoryUtil;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.Unique;

import java.nio.ByteOrder;

@Mixin(BufferBuilder.class)
public abstract class BufferBuilderM implements ExtendedVertexBuilder, VertexConsumer {
    private static final boolean IS_LITTLE_ENDIAN = ByteOrder.nativeOrder() == ByteOrder.LITTLE_ENDIAN;
    private static final int MAX_VERTICES = 16777215;

    @Shadow @Final private ByteBufferBuilder buffer;
    @Shadow private long vertexPointer;
    @Shadow private int vertices;
    @Shadow @Final private VertexFormat format;
    @Shadow @Final private PrimitiveTopology primitiveTopology;
    @Shadow @Final private int vertexSize;
    @Shadow @Final private boolean blockFormat;
    @Shadow @Final private boolean entityFormat;

    /** Created on first use: field initializers aren't applied to every BufferBuilder */
    @Unique private Vector3f scratch;

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
    public long ruthenium$vertexPointer() {
        return this.vertexPointer;
    }

    @Override
    public long ruthenium$beginElement(int semanticId) {
        return this.beginElement(semanticId);
    }

    /**
     * Writes the quad's four vertices with one reservation for the block and entity formats. Same values as vanilla's
     * default method, which this calls otherwise: the same position and normal transforms (into a scratch vector
     * instead of new ones) and the same color, UV, overlay and light per vertex. The normal's padding byte is written
     * as 0, which vanilla leaves as it is.
     */
    @Override
    public void putBakedQuad(PoseStack.Pose pose, BakedQuad quad, QuadInstance instance) {
        if (!(this.entityFormat || this.blockFormat) || !IS_LITTLE_ENDIAN || !Initializer.CONFIG.directBakedQuads) {
            VertexConsumer.super.putBakedQuad(pose, quad, instance);
            return;
        }

        this.writeBakedQuad(this.reserveVertices(4), pose, quad, instance);
    }

    @Unique
    private void writeBakedQuad(long ptr, PoseStack.Pose pose, BakedQuad quad, QuadInstance instance) {
        Vector3f scratch = this.scratch;
        if (scratch == null) {
            scratch = this.scratch = new Vector3f();
        }
        int lightEmission = quad.materialInfo().lightEmission();
        int packedNormal = 0;
        if (this.entityFormat) {
            Vector3f normal = pose.transformNormal(quad.direction().getUnitVec3f(), scratch);
            packedNormal = (normalByte(normal.x()) & 0xFF)
                    | (normalByte(normal.y()) & 0xFF) << 8
                    | (normalByte(normal.z()) & 0xFF) << 16;
        }

        Matrix4f matrix = pose.pose();
        for (int vertex = 0; vertex < 4; vertex++) {
            Vector3f pos = matrix.transformPosition(quad.position(vertex), scratch);
            long packedUv = quad.packedUV(vertex);
            int light = instance.getLightCoordsWithEmission(vertex, lightEmission);

            MemoryUtil.memPutFloat(ptr, pos.x());
            MemoryUtil.memPutFloat(ptr + 4, pos.y());
            MemoryUtil.memPutFloat(ptr + 8, pos.z());
            MemoryUtil.memPutInt(ptr + 12, ARGB.toABGR(instance.getColor(vertex)));
            MemoryUtil.memPutFloat(ptr + 16, UVPair.unpackU(packedUv));
            MemoryUtil.memPutFloat(ptr + 20, UVPair.unpackV(packedUv));
            if (this.entityFormat) {
                MemoryUtil.memPutInt(ptr + 24, instance.overlayCoords());
                MemoryUtil.memPutInt(ptr + 28, light);
                MemoryUtil.memPutInt(ptr + 32, packedNormal);
            } else {
                MemoryUtil.memPutInt(ptr + 24, light);
            }
            ptr += this.vertexSize;
        }
    }

    /** Same conversion as vanilla's BufferBuilder.normalIntValue */
    @Unique
    private static byte normalByte(float c) {
        return (byte) ((int) (Mth.clamp(c, -1.0F, 1.0F) * 127.0F) & 0xFF);
    }
}
