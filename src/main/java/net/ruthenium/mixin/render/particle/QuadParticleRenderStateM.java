package net.ruthenium.mixin.render.particle;

import com.mojang.blaze3d.vertex.DefaultVertexFormat;
import com.mojang.blaze3d.vertex.VertexConsumer;
import net.minecraft.client.renderer.state.level.QuadParticleRenderState;
import net.minecraft.util.ARGB;
import net.ruthenium.interfaces.ExtendedVertexBuilder;
import org.joml.Quaternionf;
import org.joml.Vector3f;
import org.lwjgl.system.MemoryUtil;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Overwrite;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.Unique;

/**
 * Writes particle quads directly into a particle-format BufferBuilder: the rotation is applied to the two quad
 * axes once instead of to each corner, and no objects are allocated per particle.
 */
@Mixin(QuadParticleRenderState.class)
public abstract class QuadParticleRenderStateM {
    @Unique private static final int VERTEX_SIZE = 28;

    @Unique private final Quaternionf rotation = new Quaternionf();
    @Unique private final Vector3f right = new Vector3f();
    @Unique private final Vector3f up = new Vector3f();

    @Shadow
    protected abstract void renderVertex(VertexConsumer builder, Quaternionf rotation, float x, float y, float z,
                                         float nx, float ny, float scale, float u, float v, int color, int lightCoords);

    /**
     * @author Ruthenium
     * @reason Fast path for particle-format buffers; an overwrite rather than an inject so that no CallbackInfo is
     * allocated per particle
     */
    @Overwrite
    protected void renderRotatedQuad(VertexConsumer builder, float x, float y, float z,
                                     float xRot, float yRot, float zRot, float wRot, float scale,
                                     float u0, float u1, float v0, float v1, int color, int lightCoords) {
        ExtendedVertexBuilder vertexBuilder = ExtendedVertexBuilder.of(builder, DefaultVertexFormat.PARTICLE);
        if (vertexBuilder == null) {
            // Vanilla
            Quaternionf rotation = new Quaternionf(xRot, yRot, zRot, wRot);
            this.renderVertex(builder, rotation, x, y, z, 1.0F, -1.0F, scale, u1, v1, color, lightCoords);
            this.renderVertex(builder, rotation, x, y, z, 1.0F, 1.0F, scale, u1, v0, color, lightCoords);
            this.renderVertex(builder, rotation, x, y, z, -1.0F, 1.0F, scale, u0, v0, color, lightCoords);
            this.renderVertex(builder, rotation, x, y, z, -1.0F, -1.0F, scale, u0, v1, color, lightCoords);
            return;
        }

        this.rotation.set(xRot, yRot, zRot, wRot);
        Vector3f right = this.rotation.transform(1.0F, 0.0F, 0.0F, this.right).mul(scale);
        Vector3f up = this.rotation.transform(0.0F, 1.0F, 0.0F, this.up).mul(scale);

        int abgr = ARGB.toABGR(color);
        long ptr = vertexBuilder.reserveVertices(4);

        putVertex(ptr, x + right.x - up.x, y + right.y - up.y, z + right.z - up.z, u1, v1, abgr, lightCoords);
        putVertex(ptr + VERTEX_SIZE, x + right.x + up.x, y + right.y + up.y, z + right.z + up.z, u1, v0, abgr, lightCoords);
        putVertex(ptr + VERTEX_SIZE * 2, x - right.x + up.x, y - right.y + up.y, z - right.z + up.z, u0, v0, abgr, lightCoords);
        putVertex(ptr + VERTEX_SIZE * 3, x - right.x - up.x, y - right.y - up.y, z - right.z - up.z, u0, v1, abgr, lightCoords);
    }

    /** PARTICLE format: Position (3 floats), UV0 (2 floats), Color (RGBA8), UV2 (2 shorts) */
    @Unique
    private static void putVertex(long ptr, float x, float y, float z, float u, float v, int abgr, int light) {
        MemoryUtil.memPutFloat(ptr, x);
        MemoryUtil.memPutFloat(ptr + 4, y);
        MemoryUtil.memPutFloat(ptr + 8, z);
        MemoryUtil.memPutFloat(ptr + 12, u);
        MemoryUtil.memPutFloat(ptr + 16, v);
        MemoryUtil.memPutInt(ptr + 20, abgr);
        MemoryUtil.memPutInt(ptr + 24, light);
    }
}
