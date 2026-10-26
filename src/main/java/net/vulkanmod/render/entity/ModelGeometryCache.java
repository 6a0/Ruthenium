package net.vulkanmod.render.entity;

import com.mojang.blaze3d.vertex.DefaultVertexFormat;
import com.mojang.blaze3d.vertex.PoseStack;
import com.mojang.blaze3d.vertex.VertexConsumer;
import it.unimi.dsi.fastutil.objects.Reference2ObjectOpenHashMap;
import it.unimi.dsi.fastutil.objects.ReferenceOpenHashSet;
import net.minecraft.client.model.Model;
import net.minecraft.client.model.object.book.BookModel;
import net.minecraft.client.renderer.texture.UvMapping;
import net.minecraft.util.ARGB;
import net.vulkanmod.interfaces.ExtendedVertexBuilder;
import org.joml.Matrix3f;
import org.joml.Matrix4f;
import org.joml.Vector3f;
import org.lwjgl.system.MemoryUtil;

import java.util.Arrays;

/**
 * Reuses the vertices of models in their rest pose across submits.
 * <p>
 * Block entities like chests submit the same model in the same few states over and over (closed lid, four
 * facings, a few textures); their vertices only differ by the pose's translation and the light. For models whose
 * state is a {@link Float} at 0 (rest pose), this captures the model once per pose rotation, overlay, tint and
 * texture mapping, with the vanilla code path and without the translation, and afterwards writes the captured
 * vertices with the translation added and the submit's light. Back faces are skipped as in
 * {@link ModelCulling}, with each face's plane precomputed.
 * <p>
 * Models whose geometry is fixed but whose pose keeps changing (closed enchanting table books spin and bob) are
 * captured once in model space instead ({@link #modelSpaceKey}) and their distinct corners are transformed by
 * the submit's pose on every write.
 * <p>
 * The only difference to vanilla is float rounding: vanilla folds the translation (or the whole submit pose)
 * into the part matrices before transforming, this applies it after (~1e-5 blocks at render distance). Only used
 * from the render thread.
 */
public final class ModelGeometryCache {
    private static final int VERTEX_SIZE = 36;
    private static final int MAX_ENTRIES_PER_MODEL = 16;
    private static final int MAX_MODELS = 256;
    private static final float EYE_MARGIN_SQ = ModelCulling.EYE_MARGIN * ModelCulling.EYE_MARGIN;

    /** Model space key of closed books: with openness 0 the page flips don't move anything */
    private static final Object BOOK_CLOSED = new Object();

    private static final Reference2ObjectOpenHashMap<Model<?>, Entry[]> ENTRIES = new Reference2ObjectOpenHashMap<>();
    private static final Reference2ObjectOpenHashMap<Model<?>, ModelSpaceEntry[]> MODEL_SPACE_ENTRIES =
            new Reference2ObjectOpenHashMap<>();
    private static final ReferenceOpenHashSet<Model<?>> UNCACHEABLE = new ReferenceOpenHashSet<>();
    private static final PoseStack CAPTURE_POSE = new PoseStack();
    private static final Capture CAPTURE = new Capture();
    private static boolean[] visible = new boolean[64];
    private static float[] transformedCorners = new float[64 * 3];
    private static final Vector3f SCRATCH = new Vector3f();

    private ModelGeometryCache() {
    }

    public static boolean isCacheable(Object state) {
        return state instanceof Float f && f.floatValue() == 0.0f && Float.floatToRawIntBits(f) == 0;
    }

    /**
     * Writes a rest pose model into {@code buffer} from the cache, capturing it first if needed.
     *
     * @return false if the model was not written and has to be rendered the vanilla way
     */
    public static <S> boolean write(Model<? super S> model, S state, PoseStack.Pose pose, VertexConsumer buffer,
                                    int lightCoords, int overlayCoords, int color, UvMapping uvMapping,
                                    boolean cullBackFaces) {
        ExtendedVertexBuilder vertexBuilder = ExtendedVertexBuilder.of(buffer, DefaultVertexFormat.ENTITY);
        Matrix4f matrix = pose.pose();
        if (vertexBuilder == null || !isAffine(matrix) || UNCACHEABLE.contains(model)) {
            return false;
        }

        Entry entry = find(model, pose, overlayCoords, color, uvMapping);
        if (entry == null) {
            entry = capture(model, state, pose, overlayCoords, color, uvMapping);
            if (entry == null) {
                return false;
            }
        }

        entry.write(vertexBuilder, matrix.m30(), matrix.m31(), matrix.m32(), lightCoords, cullBackFaces);
        return true;
    }

    /**
     * The key under which a state's geometry can be captured in model space, or null. Only states known to give
     * the same geometry are folded together.
     */
    public static Object modelSpaceKey(Object state) {
        if (state instanceof BookModel.State book && book.openness() == 0.0f) {
            return BOOK_CLOSED;
        }
        return null;
    }

    /**
     * Writes a model whose state has a {@link #modelSpaceKey} from the cache, transformed by {@code pose},
     * capturing it first if needed.
     *
     * @return false if the model was not written and has to be rendered the vanilla way
     */
    public static <S> boolean writeModelSpace(Model<? super S> model, S state, Object key, PoseStack.Pose pose,
                                              VertexConsumer buffer, int lightCoords, int overlayCoords, int color,
                                              UvMapping uvMapping, boolean cullBackFaces) {
        ExtendedVertexBuilder vertexBuilder = ExtendedVertexBuilder.of(buffer, DefaultVertexFormat.ENTITY);
        if (vertexBuilder == null || UNCACHEABLE.contains(model)) {
            return false;
        }

        ModelSpaceEntry entry = findModelSpace(model, key, overlayCoords, color, uvMapping);
        if (entry == null) {
            entry = captureModelSpace(model, state, key, overlayCoords, color, uvMapping);
            if (entry == null) {
                return false;
            }
        }

        entry.write(vertexBuilder, pose, lightCoords, cullBackFaces);
        return true;
    }

    public static void invalidate() {
        ENTRIES.clear();
        MODEL_SPACE_ENTRIES.clear();
        UNCACHEABLE.clear();
    }

    private static ModelSpaceEntry findModelSpace(Model<?> model, Object key, int overlay, int color,
                                                  UvMapping uvMapping) {
        ModelSpaceEntry[] entries = MODEL_SPACE_ENTRIES.get(model);
        if (entries == null) {
            return null;
        }

        for (ModelSpaceEntry entry : entries) {
            if (entry == null) {
                break;
            }
            if (entry.key == key && entry.uvMapping == uvMapping && entry.overlay == overlay && entry.color == color) {
                return entry;
            }
        }
        return null;
    }

    private static <S> ModelSpaceEntry captureModelSpace(Model<? super S> model, S state, Object key, int overlay,
                                                         int color, UvMapping uvMapping) {
        ModelSpaceEntry[] entries = MODEL_SPACE_ENTRIES.get(model);
        int slot = 0;
        if (entries == null) {
            if (MODEL_SPACE_ENTRIES.size() >= MAX_MODELS) {
                invalidate();
            }
            entries = new ModelSpaceEntry[MAX_ENTRIES_PER_MODEL];
            MODEL_SPACE_ENTRIES.put(model, entries);
        } else {
            while (slot < entries.length && entries[slot] != null) {
                slot++;
            }
            if (slot == entries.length) {
                return null;
            }
        }

        CAPTURE_POSE.last().setIdentity();
        Capture capture = CAPTURE;
        capture.reset();
        VertexConsumer target = uvMapping != null ? uvMapping.wrap(capture) : capture;
        model.setupAnim(state);
        model.renderToBuffer(CAPTURE_POSE, target, 0, overlay, color);

        ModelSpaceEntry entry = capture.invalid || capture.vertexCount % 4 != 0
                ? null : ModelSpaceEntry.create(key, overlay, color, uvMapping, capture);
        if (entry == null) {
            UNCACHEABLE.add(model);
            return null;
        }

        entries[slot] = entry;
        return entry;
    }

    private static Entry find(Model<?> model, PoseStack.Pose pose, int overlay, int color, UvMapping uvMapping) {
        Entry[] entries = ENTRIES.get(model);
        if (entries == null) {
            return null;
        }

        for (Entry entry : entries) {
            if (entry == null) {
                break;
            }
            if (entry.matches(pose, overlay, color, uvMapping)) {
                return entry;
            }
        }
        return null;
    }

    private static <S> Entry capture(Model<? super S> model, S state, PoseStack.Pose pose, int overlay, int color,
                                     UvMapping uvMapping) {
        Entry[] entries = ENTRIES.get(model);
        int slot = 0;
        if (entries == null) {
            if (ENTRIES.size() >= MAX_MODELS) {
                invalidate();
            }
            entries = new Entry[MAX_ENTRIES_PER_MODEL];
            ENTRIES.put(model, entries);
        } else {
            while (slot < entries.length && entries[slot] != null) {
                slot++;
            }
            if (slot == entries.length) {
                // Not a handful of rest poses after all
                return null;
            }
        }

        PoseStack.Pose capturePose = CAPTURE_POSE.last();
        capturePose.set(pose);
        capturePose.pose().setTranslation(0.0f, 0.0f, 0.0f);

        Capture capture = CAPTURE;
        capture.reset();
        VertexConsumer target = uvMapping != null ? uvMapping.wrap(capture) : capture;
        model.setupAnim(state);
        // Light is replaced per write
        model.renderToBuffer(CAPTURE_POSE, target, 0, overlay, color);

        if (capture.invalid || capture.vertexCount % 4 != 0) {
            UNCACHEABLE.add(model);
            return null;
        }

        Entry entry = new Entry(pose, overlay, color, uvMapping, capture);
        entries[slot] = entry;
        return entry;
    }

    private static boolean isAffine(Matrix4f m) {
        return m.m03() == 0.0f && m.m13() == 0.0f && m.m23() == 0.0f && m.m33() == 1.0f;
    }

    private static final class Entry {
        private final float[] linear = new float[9];
        private final float[] normalMatrix = new float[9];
        private final int overlay;
        private final int color;
        private final UvMapping uvMapping;

        private final int quadCount;
        /** x y z u v per vertex */
        private final float[] vertices;
        private final int[] colors;
        private final int[] normals;
        /** Per quad: geometric normal xyz (unnormalized), its dot with the first vertex, and its squared length */
        private final float[] planes;

        Entry(PoseStack.Pose pose, int overlay, int color, UvMapping uvMapping, Capture capture) {
            storeKey(pose, this.linear, this.normalMatrix);
            this.overlay = overlay;
            this.color = color;
            this.uvMapping = uvMapping;

            int vertexCount = capture.vertexCount;
            this.quadCount = vertexCount / 4;
            this.vertices = Arrays.copyOf(capture.vertices, vertexCount * 5);
            this.colors = Arrays.copyOf(capture.colors, vertexCount);
            this.normals = Arrays.copyOf(capture.normals, vertexCount);
            this.planes = new float[this.quadCount * 5];

            float[] v = this.vertices;
            for (int q = 0; q < this.quadCount; q++) {
                int i0 = q * 20, i1 = i0 + 5, i2 = i0 + 10;
                float ax = v[i1] - v[i0], ay = v[i1 + 1] - v[i0 + 1], az = v[i1 + 2] - v[i0 + 2];
                float bx = v[i2] - v[i0], by = v[i2 + 1] - v[i0 + 1], bz = v[i2 + 2] - v[i0 + 2];
                float nx = ay * bz - az * by;
                float ny = az * bx - ax * bz;
                float nz = ax * by - ay * bx;
                this.planes[q * 5] = nx;
                this.planes[q * 5 + 1] = ny;
                this.planes[q * 5 + 2] = nz;
                this.planes[q * 5 + 3] = nx * v[i0] + ny * v[i0 + 1] + nz * v[i0 + 2];
                this.planes[q * 5 + 4] = nx * nx + ny * ny + nz * nz;
            }
        }

        boolean matches(PoseStack.Pose pose, int overlay, int color, UvMapping uvMapping) {
            if (this.uvMapping != uvMapping || this.overlay != overlay || this.color != color) {
                return false;
            }

            Matrix4f m = pose.pose();
            float[] l = this.linear;
            if (l[0] != m.m00() || l[1] != m.m01() || l[2] != m.m02()
                    || l[3] != m.m10() || l[4] != m.m11() || l[5] != m.m12()
                    || l[6] != m.m20() || l[7] != m.m21() || l[8] != m.m22()) {
                return false;
            }

            Matrix3f n = pose.normal();
            float[] k = this.normalMatrix;
            return k[0] == n.m00() && k[1] == n.m01() && k[2] == n.m02()
                    && k[3] == n.m10() && k[4] == n.m11() && k[5] == n.m12()
                    && k[6] == n.m20() && k[7] == n.m21() && k[8] == n.m22();
        }

        void write(ExtendedVertexBuilder vertexBuilder, float tx, float ty, float tz, int light, boolean cullBackFaces) {
            int quads = this.quadCount;
            if (visible.length < quads) {
                visible = new boolean[quads * 2];
            }

            boolean[] visible = ModelGeometryCache.visible;
            float[] planes = this.planes;
            int visibleQuads = 0;
            for (int q = 0; q < quads; q++) {
                boolean keep = true;
                if (cullBackFaces) {
                    int p = q * 5;
                    // Same test as ModelPartCubeM.isBackFace, with the eye at the origin
                    float behind = planes[p + 3] + planes[p] * tx + planes[p + 1] * ty + planes[p + 2] * tz;
                    keep = !(behind > 0.0f && behind * behind >= EYE_MARGIN_SQ * planes[p + 4]);
                }
                visible[q] = keep;
                if (keep) {
                    visibleQuads++;
                }
            }

            if (visibleQuads == 0) {
                return;
            }

            long ptr = vertexBuilder.reserveVertices(visibleQuads * 4);
            float[] v = this.vertices;
            int[] colors = this.colors;
            int[] normals = this.normals;
            int overlay = this.overlay;
            for (int q = 0; q < quads; q++) {
                if (!visible[q]) {
                    continue;
                }

                for (int i = q * 4, end = i + 4; i < end; i++) {
                    int o = i * 5;
                    MemoryUtil.memPutFloat(ptr, v[o] + tx);
                    MemoryUtil.memPutFloat(ptr + 4, v[o + 1] + ty);
                    MemoryUtil.memPutFloat(ptr + 8, v[o + 2] + tz);
                    MemoryUtil.memPutInt(ptr + 12, colors[i]);
                    MemoryUtil.memPutFloat(ptr + 16, v[o + 3]);
                    MemoryUtil.memPutFloat(ptr + 20, v[o + 4]);
                    MemoryUtil.memPutInt(ptr + 24, overlay);
                    MemoryUtil.memPutInt(ptr + 28, light);
                    MemoryUtil.memPutInt(ptr + 32, normals[i]);
                    ptr += VERTEX_SIZE;
                }
            }
        }

        private static void storeKey(PoseStack.Pose pose, float[] linear, float[] normalMatrix) {
            Matrix4f m = pose.pose();
            linear[0] = m.m00(); linear[1] = m.m01(); linear[2] = m.m02();
            linear[3] = m.m10(); linear[4] = m.m11(); linear[5] = m.m12();
            linear[6] = m.m20(); linear[7] = m.m21(); linear[8] = m.m22();
            Matrix3f n = pose.normal();
            normalMatrix[0] = n.m00(); normalMatrix[1] = n.m01(); normalMatrix[2] = n.m02();
            normalMatrix[3] = n.m10(); normalMatrix[4] = n.m11(); normalMatrix[5] = n.m12();
            normalMatrix[6] = n.m20(); normalMatrix[7] = n.m21(); normalMatrix[8] = n.m22();
        }
    }

    /**
     * A model captured with the identity pose. Its distinct corners are transformed per write (as in
     * {@code ModelPartCubeM}) and each quad's normal is transformed with the submit's normal matrix.
     */
    private static final class ModelSpaceEntry {
        private final Object key;
        private final int overlay;
        private final int color;
        private final UvMapping uvMapping;

        private final int quadCount;
        /** Distinct positions, xyz interleaved */
        private final float[] corners;
        /** Index into {@link #corners} per vertex */
        private final short[] cornerIndices;
        /** u v per vertex */
        private final float[] uvs;
        private final int[] colors;
        /** Model space normal per quad */
        private final float[] normals;

        private ModelSpaceEntry(Object key, int overlay, int color, UvMapping uvMapping, int quadCount,
                                float[] corners, short[] cornerIndices, float[] uvs, int[] colors, float[] normals) {
            this.key = key;
            this.overlay = overlay;
            this.color = color;
            this.uvMapping = uvMapping;
            this.quadCount = quadCount;
            this.corners = corners;
            this.cornerIndices = cornerIndices;
            this.uvs = uvs;
            this.colors = colors;
            this.normals = normals;
        }

        /** Returns null if the quads don't have one normal each */
        static ModelSpaceEntry create(Object key, int overlay, int color, UvMapping uvMapping, Capture capture) {
            int vertexCount = capture.vertexCount;
            int quadCount = vertexCount / 4;
            float[] v = capture.vertices;
            float[] n = capture.normalVectors;

            float[] normals = new float[quadCount * 3];
            for (int q = 0; q < quadCount; q++) {
                int first = q * 12;
                for (int i = first + 3; i < first + 12; i += 3) {
                    if (n[i] != n[first] || n[i + 1] != n[first + 1] || n[i + 2] != n[first + 2]) {
                        return null;
                    }
                }
                normals[q * 3] = n[first];
                normals[q * 3 + 1] = n[first + 1];
                normals[q * 3 + 2] = n[first + 2];
            }

            float[] corners = new float[vertexCount * 3];
            short[] cornerIndices = new short[vertexCount];
            float[] uvs = new float[vertexCount * 2];
            int cornerCount = 0;
            for (int i = 0; i < vertexCount; i++) {
                float x = v[i * 5], y = v[i * 5 + 1], z = v[i * 5 + 2];
                int found = -1;
                for (int c = 0; c < cornerCount; c++) {
                    if (corners[c * 3] == x && corners[c * 3 + 1] == y && corners[c * 3 + 2] == z) {
                        found = c;
                        break;
                    }
                }
                if (found == -1) {
                    found = cornerCount++;
                    corners[found * 3] = x;
                    corners[found * 3 + 1] = y;
                    corners[found * 3 + 2] = z;
                }
                cornerIndices[i] = (short) found;
                uvs[i * 2] = v[i * 5 + 3];
                uvs[i * 2 + 1] = v[i * 5 + 4];
            }

            return new ModelSpaceEntry(key, overlay, color, uvMapping, quadCount, Arrays.copyOf(corners, cornerCount * 3),
                    cornerIndices, uvs, Arrays.copyOf(capture.colors, vertexCount), normals);
        }

        void write(ExtendedVertexBuilder vertexBuilder, PoseStack.Pose pose, int light, boolean cullBackFaces) {
            float[] corners = this.corners;
            if (transformedCorners.length < corners.length) {
                transformedCorners = new float[corners.length * 2];
            }
            float[] transformed = transformedCorners;
            Vector3f scratch = SCRATCH;
            Matrix4f matrix = pose.pose();
            for (int i = 0; i < corners.length; i += 3) {
                matrix.transformPosition(corners[i], corners[i + 1], corners[i + 2], scratch);
                transformed[i] = scratch.x();
                transformed[i + 1] = scratch.y();
                transformed[i + 2] = scratch.z();
            }

            int quads = this.quadCount;
            if (visible.length < quads) {
                visible = new boolean[quads * 2];
            }

            boolean[] visible = ModelGeometryCache.visible;
            short[] cornerIndices = this.cornerIndices;
            int visibleQuads = 0;
            for (int q = 0; q < quads; q++) {
                boolean keep = !cullBackFaces || !isBackFace(transformed, cornerIndices[q * 4] * 3,
                        cornerIndices[q * 4 + 1] * 3, cornerIndices[q * 4 + 2] * 3);
                visible[q] = keep;
                if (keep) {
                    visibleQuads++;
                }
            }

            if (visibleQuads == 0) {
                return;
            }

            long ptr = vertexBuilder.reserveVertices(visibleQuads * 4);
            float[] uvs = this.uvs;
            int[] colors = this.colors;
            float[] normals = this.normals;
            int overlay = this.overlay;
            for (int q = 0; q < quads; q++) {
                if (!visible[q]) {
                    continue;
                }

                Vector3f normal = pose.transformNormal(normals[q * 3], normals[q * 3 + 1], normals[q * 3 + 2], scratch);
                int packedNormal = (Capture.normalByte(normal.x()) & 0xFF)
                        | (Capture.normalByte(normal.y()) & 0xFF) << 8
                        | (Capture.normalByte(normal.z()) & 0xFF) << 16;

                for (int i = q * 4, end = i + 4; i < end; i++) {
                    int corner = cornerIndices[i] * 3;
                    MemoryUtil.memPutFloat(ptr, transformed[corner]);
                    MemoryUtil.memPutFloat(ptr + 4, transformed[corner + 1]);
                    MemoryUtil.memPutFloat(ptr + 8, transformed[corner + 2]);
                    MemoryUtil.memPutInt(ptr + 12, colors[i]);
                    MemoryUtil.memPutFloat(ptr + 16, uvs[i * 2]);
                    MemoryUtil.memPutFloat(ptr + 20, uvs[i * 2 + 1]);
                    MemoryUtil.memPutInt(ptr + 24, overlay);
                    MemoryUtil.memPutInt(ptr + 28, light);
                    MemoryUtil.memPutInt(ptr + 32, packedNormal);
                    ptr += VERTEX_SIZE;
                }
            }
        }

        /** Same test as ModelPartCubeM.isBackFace */
        private static boolean isBackFace(float[] t, int i0, int i1, int i2) {
            float x0 = t[i0], y0 = t[i0 + 1], z0 = t[i0 + 2];
            float ax = t[i1] - x0, ay = t[i1 + 1] - y0, az = t[i1 + 2] - z0;
            float bx = t[i2] - x0, by = t[i2 + 1] - y0, bz = t[i2 + 2] - z0;
            float nx = ay * bz - az * by;
            float ny = az * bx - ax * bz;
            float nz = ax * by - ay * bx;

            float behind = nx * x0 + ny * y0 + nz * z0;
            return behind > 0.0f && behind * behind >= EYE_MARGIN_SQ * (nx * nx + ny * ny + nz * nz);
        }
    }

    /**
     * Records full entity vertices as {@code BufferBuilder} would write them (ABGR color, packed normal).
     * Model parts emit whole vertices through the bulk {@code addVertex}; anything else makes the capture fail.
     */
    private static final class Capture implements VertexConsumer {
        float[] vertices = new float[256 * 5];
        int[] colors = new int[256];
        int[] normals = new int[256];
        /** The normal as passed in, xyz per vertex */
        float[] normalVectors = new float[256 * 3];
        int vertexCount;
        boolean invalid;

        void reset() {
            this.vertexCount = 0;
            this.invalid = false;
        }

        @Override
        public void addVertex(float x, float y, float z, int color, float u, float v, int overlayCoords, int lightCoords,
                              float nx, float ny, float nz) {
            int i = this.vertexCount;
            if (i == this.colors.length) {
                this.vertices = Arrays.copyOf(this.vertices, this.vertices.length * 2);
                this.colors = Arrays.copyOf(this.colors, this.colors.length * 2);
                this.normals = Arrays.copyOf(this.normals, this.normals.length * 2);
                this.normalVectors = Arrays.copyOf(this.normalVectors, this.normalVectors.length * 2);
            }

            int o = i * 5;
            this.vertices[o] = x;
            this.vertices[o + 1] = y;
            this.vertices[o + 2] = z;
            this.vertices[o + 3] = u;
            this.vertices[o + 4] = v;
            this.colors[i] = ARGB.toABGR(color);
            this.normals[i] = (normalByte(nx) & 0xFF) | (normalByte(ny) & 0xFF) << 8 | (normalByte(nz) & 0xFF) << 16;
            this.normalVectors[i * 3] = nx;
            this.normalVectors[i * 3 + 1] = ny;
            this.normalVectors[i * 3 + 2] = nz;
            this.vertexCount = i + 1;
        }

        /** Same conversion as vanilla's BufferBuilder.normalIntValue */
        private static byte normalByte(float c) {
            return (byte) ((int) (Math.clamp(c, -1.0F, 1.0F) * 127.0F) & 0xFF);
        }

        // Partial vertex writes can't be captured
        private VertexConsumer invalid() {
            this.invalid = true;
            return this;
        }

        @Override public VertexConsumer addVertex(float x, float y, float z) { return this.invalid(); }
        @Override public VertexConsumer setColor(int r, int g, int b, int a) { return this.invalid(); }
        @Override public VertexConsumer setColor(int color) { return this.invalid(); }
        @Override public VertexConsumer setUv(float u, float v) { return this.invalid(); }
        @Override public VertexConsumer setUv1(int u, int v) { return this.invalid(); }
        @Override public VertexConsumer setUv2(int u, int v) { return this.invalid(); }
        @Override public VertexConsumer setUv3(float u, float v) { return this.invalid(); }
        @Override public VertexConsumer setNormal(float x, float y, float z) { return this.invalid(); }
        @Override public VertexConsumer setLineWidth(float width) { return this.invalid(); }
    }
}
