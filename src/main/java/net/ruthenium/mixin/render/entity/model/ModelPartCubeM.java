package net.ruthenium.mixin.render.entity.model;

import com.mojang.blaze3d.vertex.DefaultVertexFormat;
import com.mojang.blaze3d.vertex.PoseStack;
import com.mojang.blaze3d.vertex.VertexConsumer;
import net.minecraft.client.model.geom.ModelPart;
import net.minecraft.client.renderer.SpriteCoordinateExpander;
import net.minecraft.client.renderer.texture.UvMapping;
import net.minecraft.util.ARGB;
import net.ruthenium.interfaces.ExtendedVertexBuilder;
import net.ruthenium.render.entity.ModelCulling;
import net.ruthenium.render.entity.TransparentFaces;
import org.joml.Matrix4f;
import org.joml.Vector3f;
import org.lwjgl.system.MemoryUtil;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Overwrite;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.Unique;

import java.util.Arrays;

/**
 * Fast path for cubes written into an entity-format {@link com.mojang.blaze3d.vertex.BufferBuilder}:
 * the cube's distinct corners are transformed once (8 instead of 24 positions), each face is written as one
 * 4-vertex reservation, and color, light, overlay and the packed normal are computed once per face.
 * Output is identical to vanilla's {@code Cube.compile}, except that faces the GPU would cull as back faces are
 * not written when {@link ModelCulling} allows it, and neither are faces that draw nothing in the render type's
 * texture ({@link TransparentFaces}).
 * <p>
 * Like vanilla model parts this is not thread safe; models are only rendered from the render thread.
 */
@Mixin(ModelPart.Cube.class)
public class ModelPartCubeM {
    @Unique private static final int VERTEX_SIZE = 36;
    @Unique private static final float EYE_MARGIN_SQ = ModelCulling.EYE_MARGIN * ModelCulling.EYE_MARGIN;

    @Shadow @Final public ModelPart.Polygon[] polygons;

    /** Distinct vertex positions (in world units) of this cube, xyz interleaved. */
    @Unique private float[] corners;
    /** Index into {@link #corners} of each polygon vertex, 4 per polygon. */
    @Unique private byte[] cornerIndices;
    @Unique private float[] transformedCorners;
    @Unique private final Vector3f scratch = new Vector3f();
    /**
     * Hash table of texture alphas ({@link TransparentFaces}) and the polygons that draw nothing in them, bit per
     * polygon. It grows with the number of textures the cube is drawn with: a villager's cubes see ~28 per frame
     * (types, professions and levels).
     */
    @Unique private TransparentFaces.TextureAlpha[] transparentKeys;
    @Unique private int[] transparentMasks;
    @Unique private int transparentCount;
    @Unique private int transparentGeneration;

    /**
     * @author Ruthenium
     * @reason Fast path for entity-format buffers; an overwrite rather than an inject so that no CallbackInfo is
     * allocated per cube
     */
    @Overwrite
    public void compile(PoseStack.Pose pose, VertexConsumer builder, int lightCoords, int overlayCoords, int color) {
        UvMapping mapping = null;
        VertexConsumer target = builder;
        if (builder instanceof SpriteCoordinateExpander expander) {
            target = expander.delegate();
            mapping = expander.mapping();
        }

        ExtendedVertexBuilder vertexBuilder = ExtendedVertexBuilder.of(target, DefaultVertexFormat.ENTITY);
        if (vertexBuilder == null) {
            this.compileVanilla(pose, builder, lightCoords, overlayCoords, color);
            return;
        }

        // Faces that draw nothing in this render type's texture (TransparentFaces); atlas sprites aren't tested
        int skipped = 0;
        TransparentFaces.TextureAlpha textureAlpha = ModelCulling.textureAlpha();
        if (textureAlpha != null && mapping == null) {
            skipped = this.transparentPolygons(textureAlpha);
            if (skipped == (1 << this.polygons.length) - 1) {
                return;
            }
        }

        if (this.corners == null) {
            this.initCorners();
        }

        Matrix4f matrix = pose.pose();
        float[] corners = this.corners;
        float[] transformed = this.transformedCorners;
        Vector3f scratch = this.scratch;
        for (int i = 0; i < corners.length; i += 3) {
            matrix.transformPosition(corners[i], corners[i + 1], corners[i + 2], scratch);
            transformed[i] = scratch.x();
            transformed[i + 1] = scratch.y();
            transformed[i + 2] = scratch.z();
        }

        int abgr = ARGB.toABGR(color);
        boolean cullBackFaces = ModelCulling.cullBackFaces();

        for (int p = 0; p < this.polygons.length; p++) {
            if ((skipped & 1 << p) != 0 || cullBackFaces && isBackFace(transformed, p)) {
                continue;
            }

            ModelPart.Polygon polygon = this.polygons[p];
            Vector3f normal = pose.transformNormal(polygon.normal(), scratch);
            int packedNormal = (normalByte(normal.x()) & 0xFF)
                    | (normalByte(normal.y()) & 0xFF) << 8
                    | (normalByte(normal.z()) & 0xFF) << 16;

            ModelPart.Vertex[] vertices = polygon.vertices();
            long ptr = vertexBuilder.reserveVertices(4);

            for (int v = 0; v < 4; v++) {
                ModelPart.Vertex vertex = vertices[v];
                int corner = this.cornerIndices[p * 4 + v] * 3;
                float u = mapping != null ? mapping.getU(vertex.u()) : vertex.u();
                float vCoord = mapping != null ? mapping.getV(vertex.v()) : vertex.v();

                MemoryUtil.memPutFloat(ptr, transformed[corner]);
                MemoryUtil.memPutFloat(ptr + 4, transformed[corner + 1]);
                MemoryUtil.memPutFloat(ptr + 8, transformed[corner + 2]);
                MemoryUtil.memPutInt(ptr + 12, abgr);
                MemoryUtil.memPutFloat(ptr + 16, u);
                MemoryUtil.memPutFloat(ptr + 20, vCoord);
                MemoryUtil.memPutInt(ptr + 24, overlayCoords);
                MemoryUtil.memPutInt(ptr + 28, lightCoords);
                MemoryUtil.memPutInt(ptr + 32, packedNormal);
                ptr += VERTEX_SIZE;
            }
        }
    }

    /**
     * Whether the GPU would cull polygon {@code p} for every eye position within {@link ModelCulling#EYE_MARGIN} of
     * the origin. Cube faces wind counter-clockwise seen from outside, so the GPU keeps a face when
     * {@code (v1 - v0) x (v2 - v0)} points towards the eye. This uses the transformed corners, so mirroring poses
     * flip the result just like they flip the winding on screen. Degenerate faces are kept.
     */
    @Unique
    private boolean isBackFace(float[] transformed, int p) {
        int i0 = this.cornerIndices[p * 4] * 3;
        int i1 = this.cornerIndices[p * 4 + 1] * 3;
        int i2 = this.cornerIndices[p * 4 + 2] * 3;

        float x0 = transformed[i0], y0 = transformed[i0 + 1], z0 = transformed[i0 + 2];
        float ax = transformed[i1] - x0, ay = transformed[i1 + 1] - y0, az = transformed[i1 + 2] - z0;
        float bx = transformed[i2] - x0, by = transformed[i2 + 1] - y0, bz = transformed[i2 + 2] - z0;
        float nx = ay * bz - az * by;
        float ny = az * bx - ax * bz;
        float nz = ax * by - ay * bx;

        // Distance of the eye behind the face plane, times |n|
        float behind = nx * x0 + ny * y0 + nz * z0;
        return behind > 0.0f && behind * behind >= EYE_MARGIN_SQ * (nx * nx + ny * ny + nz * nz);
    }

    /** The transparent polygons of this cube in a texture, kept per texture */
    @Unique
    private int transparentPolygons(TransparentFaces.TextureAlpha textureAlpha) {
        TransparentFaces.TextureAlpha[] keys = this.transparentKeys;
        if (keys == null || this.transparentGeneration != TransparentFaces.generation()) {
            keys = this.resetTransparent(4);
        }
        int bits = Integer.numberOfTrailingZeros(keys.length);
        int slot = textureAlpha.id * 0x9E3779B9 >>> 32 - bits;
        for (TransparentFaces.TextureAlpha key; (key = keys[slot]) != null; slot = slot + 1 & keys.length - 1) {
            if (key == textureAlpha) {
                return this.transparentMasks[slot];
            }
        }

        int mask = textureAlpha.transparentPolygons(this.polygons);
        // At most half full; a table of 128 that fills up (texture alphas replaced by reloads) starts over
        if ((this.transparentCount + 1) * 2 > keys.length) {
            TransparentFaces.TextureAlpha[] oldKeys = keys;
            int[] oldMasks = this.transparentMasks;
            keys = this.resetTransparent(keys.length < 128 ? keys.length * 2 : 4);
            if (keys.length > oldKeys.length) {
                for (int i = 0; i < oldKeys.length; i++) {
                    if (oldKeys[i] != null) {
                        this.putTransparent(oldKeys[i], oldMasks[i]);
                    }
                }
            }
        }
        this.putTransparent(textureAlpha, mask);
        return mask;
    }

    @Unique
    private TransparentFaces.TextureAlpha[] resetTransparent(int capacity) {
        this.transparentKeys = new TransparentFaces.TextureAlpha[capacity];
        this.transparentMasks = new int[capacity];
        this.transparentCount = 0;
        this.transparentGeneration = TransparentFaces.generation();
        return this.transparentKeys;
    }

    @Unique
    private void putTransparent(TransparentFaces.TextureAlpha textureAlpha, int mask) {
        TransparentFaces.TextureAlpha[] keys = this.transparentKeys;
        int slot = textureAlpha.id * 0x9E3779B9 >>> 32 - Integer.numberOfTrailingZeros(keys.length);
        while (keys[slot] != null) {
            slot = slot + 1 & keys.length - 1;
        }
        keys[slot] = textureAlpha;
        this.transparentMasks[slot] = mask;
        this.transparentCount++;
    }

    /** Vanilla's Cube.compile */
    @Unique
    private void compileVanilla(PoseStack.Pose pose, VertexConsumer builder, int lightCoords, int overlayCoords, int color) {
        Matrix4f matrix = pose.pose();
        Vector3f scratchVector = new Vector3f();

        for (ModelPart.Polygon polygon : this.polygons) {
            Vector3f normal = pose.transformNormal(polygon.normal(), scratchVector);
            float nx = normal.x();
            float ny = normal.y();
            float nz = normal.z();

            for (ModelPart.Vertex vertex : polygon.vertices()) {
                Vector3f pos = matrix.transformPosition(vertex.worldX(), vertex.worldY(), vertex.worldZ(), scratchVector);
                builder.addVertex(pos.x(), pos.y(), pos.z(), color, vertex.u(), vertex.v(), overlayCoords, lightCoords, nx, ny, nz);
            }
        }
    }

    @Unique
    private void initCorners() {
        float[] unique = new float[this.polygons.length * 4 * 3];
        byte[] indices = new byte[this.polygons.length * 4];
        int count = 0;

        for (int p = 0; p < this.polygons.length; p++) {
            ModelPart.Vertex[] vertices = this.polygons[p].vertices();
            for (int v = 0; v < 4; v++) {
                float x = vertices[v].worldX(), y = vertices[v].worldY(), z = vertices[v].worldZ();

                int found = -1;
                for (int i = 0; i < count; i++) {
                    if (unique[i * 3] == x && unique[i * 3 + 1] == y && unique[i * 3 + 2] == z) {
                        found = i;
                        break;
                    }
                }
                if (found == -1) {
                    found = count++;
                    unique[found * 3] = x;
                    unique[found * 3 + 1] = y;
                    unique[found * 3 + 2] = z;
                }
                indices[p * 4 + v] = (byte) found;
            }
        }

        this.transformedCorners = new float[count * 3];
        this.cornerIndices = indices;
        this.corners = Arrays.copyOf(unique, count * 3);
    }

    /** Same conversion as vanilla's BufferBuilder.normalIntValue */
    @Unique
    private static byte normalByte(float c) {
        return (byte) ((int) (Math.clamp(c, -1.0F, 1.0F) * 127.0F) & 0xFF);
    }
}
