package net.rubidium.render.entity;

import com.mojang.blaze3d.vertex.DefaultVertexFormat;
import com.mojang.blaze3d.vertex.PoseStack;
import com.mojang.blaze3d.vertex.VertexConsumer;
import it.unimi.dsi.fastutil.objects.Reference2ObjectOpenHashMap;
import it.unimi.dsi.fastutil.objects.ReferenceOpenHashSet;
import net.minecraft.client.model.Model;
import net.minecraft.client.model.object.book.BookModel;
import net.minecraft.client.renderer.texture.UvMapping;
import net.minecraft.util.ARGB;
import net.rubidium.interfaces.ExtendedVertexBuilder;
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
    private static int[] cornerStamps = new int[64];
    private static int cornerStamp;
    private static long[] transformedXY = new long[64];
    private static long[] transformedZ = new long[64];
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
                                              UvMapping uvMapping, boolean cullBackFaces, boolean skipDuplicates) {
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

        entry.write(vertexBuilder, pose, lightCoords, cullBackFaces, skipDuplicates);
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
        /** x y z per vertex */
        private final float[] positions;
        /** u (low half) and v (high half) bits per vertex, as written */
        private final long[] uvs;
        /** ABGR color per vertex, in the high half */
        private final long[] colors;
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
            this.positions = new float[vertexCount * 3];
            this.uvs = new long[vertexCount];
            this.colors = new long[vertexCount];
            this.normals = Arrays.copyOf(capture.normals, vertexCount);
            this.planes = new float[this.quadCount * 5];
            for (int i = 0; i < vertexCount; i++) {
                System.arraycopy(capture.vertices, i * 5, this.positions, i * 3, 3);
                this.uvs[i] = pack(Float.floatToRawIntBits(capture.vertices[i * 5 + 3]),
                        Float.floatToRawIntBits(capture.vertices[i * 5 + 4]));
                this.colors[i] = pack(0, capture.colors[i]);
            }

            float[] v = this.positions;
            for (int q = 0; q < this.quadCount; q++) {
                int i0 = q * 12, i1 = i0 + 3, i2 = i0 + 6;
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
            float[] v = this.positions;
            long[] uvs = this.uvs;
            long[] colors = this.colors;
            int[] normals = this.normals;
            long overlayLight = pack(this.overlay, light);
            for (int q = 0; q < quads; q++) {
                if (!visible[q]) {
                    continue;
                }

                for (int i = q * 4, end = i + 4; i < end; i++) {
                    int o = i * 3;
                    MemoryUtil.memPutLong(ptr, pack(Float.floatToRawIntBits(v[o] + tx), Float.floatToRawIntBits(v[o + 1] + ty)));
                    MemoryUtil.memPutLong(ptr + 8, pack(Float.floatToRawIntBits(v[o + 2] + tz), 0) | colors[i]);
                    MemoryUtil.memPutLong(ptr + 16, uvs[i]);
                    MemoryUtil.memPutLong(ptr + 24, overlayLight);
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
     * A model captured with the identity pose, written with the submit's pose.
     * <p>
     * Back faces are found in model space: when the pose's linear part is a similarity (rotation and uniform
     * scale, possibly mirrored) the eye is moved into model space once and each quad is tested against its
     * precomputed plane; otherwise the corners are transformed first and tested as in {@code ModelPartCubeM}. Only
     * the corners of written quads are transformed. Quads that repeat an earlier quad exactly (the two flip pages of
     * a closed book) are skipped when the render type allows it ({@link ModelCulling#skipDuplicateQuads}).
     */
    private static final class ModelSpaceEntry {
        /** Tolerance, relative to the squared scale, of the similarity test */
        private static final float SIMILARITY_TOLERANCE = 1.0e-4f;

        private final Object key;
        private final int overlay;
        private final int color;
        private final UvMapping uvMapping;

        private final int quadCount;
        /** Distinct positions, xyz interleaved */
        private final float[] corners;
        /** Index into {@link #corners} per vertex */
        private final short[] cornerIndices;
        /** u (low half) and v (high half) bits per vertex, as written */
        private final long[] uvs;
        /** ABGR color per vertex, in the high half */
        private final long[] colors;
        /** Model space normal per quad */
        private final float[] normals;
        /** Per quad: winding normal xyz (unnormalized), its dot with the first corner, and its squared length */
        private final float[] planes;
        private final boolean[] duplicates;

        private ModelSpaceEntry(Object key, int overlay, int color, UvMapping uvMapping, int quadCount,
                                float[] corners, short[] cornerIndices, long[] uvs, long[] colors, float[] normals,
                                float[] planes, boolean[] duplicates) {
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
            this.planes = planes;
            this.duplicates = duplicates;
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
            long[] uvs = new long[vertexCount];
            long[] colors = new long[vertexCount];
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
                uvs[i] = pack(Float.floatToRawIntBits(v[i * 5 + 3]), Float.floatToRawIntBits(v[i * 5 + 4]));
                colors[i] = pack(0, capture.colors[i]);
            }

            float[] planes = new float[quadCount * 5];
            boolean[] duplicates = new boolean[quadCount];
            for (int q = 0; q < quadCount; q++) {
                int i0 = cornerIndices[q * 4] * 3, i1 = cornerIndices[q * 4 + 1] * 3, i2 = cornerIndices[q * 4 + 2] * 3;
                float ax = corners[i1] - corners[i0], ay = corners[i1 + 1] - corners[i0 + 1], az = corners[i1 + 2] - corners[i0 + 2];
                float bx = corners[i2] - corners[i0], by = corners[i2 + 1] - corners[i0 + 1], bz = corners[i2 + 2] - corners[i0 + 2];
                float nx = ay * bz - az * by;
                float ny = az * bx - ax * bz;
                float nz = ax * by - ay * bx;
                planes[q * 5] = nx;
                planes[q * 5 + 1] = ny;
                planes[q * 5 + 2] = nz;
                planes[q * 5 + 3] = nx * corners[i0] + ny * corners[i0 + 1] + nz * corners[i0 + 2];
                planes[q * 5 + 4] = nx * nx + ny * ny + nz * nz;

                for (int p = 0; p < q && !duplicates[q]; p++) {
                    duplicates[q] = !duplicates[p] && sameQuad(p, q, cornerIndices, uvs, colors, normals);
                }
            }

            return new ModelSpaceEntry(key, overlay, color, uvMapping, quadCount, Arrays.copyOf(corners, cornerCount * 3),
                    cornerIndices, uvs, colors, normals, planes, duplicates);
        }

        private static boolean sameQuad(int p, int q, short[] cornerIndices, long[] uvs, long[] colors, float[] normals) {
            for (int k = 0; k < 4; k++) {
                int a = p * 4 + k, b = q * 4 + k;
                if (cornerIndices[a] != cornerIndices[b] || uvs[a] != uvs[b] || colors[a] != colors[b]) {
                    return false;
                }
            }
            return normals[p * 3] == normals[q * 3] && normals[p * 3 + 1] == normals[q * 3 + 1]
                    && normals[p * 3 + 2] == normals[q * 3 + 2];
        }

        void write(ExtendedVertexBuilder vertexBuilder, PoseStack.Pose pose, int light, boolean cullBackFaces,
                   boolean skipDuplicates) {
            Matrix4f m = pose.pose();
            float m00 = m.m00(), m01 = m.m01(), m02 = m.m02();
            float m10 = m.m10(), m11 = m.m11(), m12 = m.m12();
            float m20 = m.m20(), m21 = m.m21(), m22 = m.m22();
            float m30 = m.m30(), m31 = m.m31(), m32 = m.m32();

            int quads = this.quadCount;
            if (visible.length < quads) {
                visible = new boolean[quads * 2];
            }
            float[] corners = this.corners;
            int cornerCount = corners.length / 3;
            if (cornerStamps.length < cornerCount) {
                cornerStamps = new int[cornerCount * 2];
                transformedXY = new long[cornerCount * 2];
                transformedZ = new long[cornerCount * 2];
            }

            boolean[] visible = ModelGeometryCache.visible;
            boolean[] duplicates = this.duplicates;
            short[] cornerIndices = this.cornerIndices;
            int[] stamps = cornerStamps;
            long[] xy = transformedXY;
            long[] zs = transformedZ;
            int stamp = nextCornerStamp();

            float s2 = m00 * m00 + m01 * m01 + m02 * m02;
            float tolerance = SIMILARITY_TOLERANCE * s2;
            boolean similarity = s2 > 0.0f
                    && Math.abs(m10 * m10 + m11 * m11 + m12 * m12 - s2) <= tolerance
                    && Math.abs(m20 * m20 + m21 * m21 + m22 * m22 - s2) <= tolerance
                    && Math.abs(m00 * m10 + m01 * m11 + m02 * m12) <= tolerance
                    && Math.abs(m00 * m20 + m01 * m21 + m02 * m22) <= tolerance
                    && Math.abs(m10 * m20 + m11 * m21 + m12 * m22) <= tolerance;

            int visibleQuads = 0;
            if (!cullBackFaces) {
                for (int q = 0; q < quads; q++) {
                    boolean keep = !(skipDuplicates && duplicates[q]);
                    visible[q] = keep;
                    visibleQuads += keep ? 1 : 0;
                }
            } else if (similarity) {
                // The eye (the origin) in model space: L^-1 (-t), with L^-1 = L^T / s2
                float ex = -(m00 * m30 + m01 * m31 + m02 * m32) / s2;
                float ey = -(m10 * m30 + m11 * m31 + m12 * m32) / s2;
                float ez = -(m20 * m30 + m21 * m31 + m22 * m32) / s2;
                // The world winding normal is cof(L) n = det(L) L^-T n, so the side flips with det(L)
                float det = m00 * (m11 * m22 - m12 * m21) - m10 * (m01 * m22 - m02 * m21) + m20 * (m01 * m12 - m02 * m11);
                float sign = det < 0.0f ? -1.0f : 1.0f;
                // World distance behind the plane = sqrt(s2) * behind / |n|
                float marginSq = EYE_MARGIN_SQ / s2;

                float[] planes = this.planes;
                for (int q = 0; q < quads; q++) {
                    int p = q * 5;
                    float behind = sign * (planes[p + 3] - (planes[p] * ex + planes[p + 1] * ey + planes[p + 2] * ez));
                    boolean keep = !(behind > 0.0f && behind * behind >= marginSq * planes[p + 4])
                            && !(skipDuplicates && duplicates[q]);
                    visible[q] = keep;
                    visibleQuads += keep ? 1 : 0;
                }
            } else {
                // Skewed or non-uniformly scaled: test the transformed corners
                for (int c = 0; c < cornerCount; c++) {
                    transformCorner(c, corners, m00, m01, m02, m10, m11, m12, m20, m21, m22, m30, m31, m32, xy, zs);
                    stamps[c] = stamp;
                }
                for (int q = 0; q < quads; q++) {
                    boolean keep = !isBackFace(xy, zs, cornerIndices[q * 4], cornerIndices[q * 4 + 1], cornerIndices[q * 4 + 2])
                            && !(skipDuplicates && duplicates[q]);
                    visible[q] = keep;
                    visibleQuads += keep ? 1 : 0;
                }
            }

            if (visibleQuads == 0) {
                return;
            }

            long ptr = vertexBuilder.reserveVertices(visibleQuads * 4);
            long[] uvs = this.uvs;
            long[] colors = this.colors;
            float[] normals = this.normals;
            long overlayLight = pack(this.overlay, light);
            Vector3f scratch = SCRATCH;
            for (int q = 0; q < quads; q++) {
                if (!visible[q]) {
                    continue;
                }

                Vector3f normal = pose.transformNormal(normals[q * 3], normals[q * 3 + 1], normals[q * 3 + 2], scratch);
                int packedNormal = (Capture.normalByte(normal.x()) & 0xFF)
                        | (Capture.normalByte(normal.y()) & 0xFF) << 8
                        | (Capture.normalByte(normal.z()) & 0xFF) << 16;

                for (int i = q * 4, end = i + 4; i < end; i++) {
                    int c = cornerIndices[i];
                    if (stamps[c] != stamp) {
                        transformCorner(c, corners, m00, m01, m02, m10, m11, m12, m20, m21, m22, m30, m31, m32, xy, zs);
                        stamps[c] = stamp;
                    }
                    MemoryUtil.memPutLong(ptr, xy[c]);
                    MemoryUtil.memPutLong(ptr + 8, zs[c] | colors[i]);
                    MemoryUtil.memPutLong(ptr + 16, uvs[i]);
                    MemoryUtil.memPutLong(ptr + 24, overlayLight);
                    MemoryUtil.memPutInt(ptr + 32, packedNormal);
                    ptr += VERTEX_SIZE;
                }
            }
        }

        /** Stores the transformed corner as written: x and y bits in {@code xy}, z bits in the low half of {@code zs} */
        private static void transformCorner(int c, float[] corners, float m00, float m01, float m02, float m10, float m11,
                                            float m12, float m20, float m21, float m22, float m30, float m31, float m32,
                                            long[] xy, long[] zs) {
            float x = corners[c * 3], y = corners[c * 3 + 1], z = corners[c * 3 + 2];
            float tx = m00 * x + m10 * y + m20 * z + m30;
            float ty = m01 * x + m11 * y + m21 * z + m31;
            float tz = m02 * x + m12 * y + m22 * z + m32;
            xy[c] = pack(Float.floatToRawIntBits(tx), Float.floatToRawIntBits(ty));
            zs[c] = pack(Float.floatToRawIntBits(tz), 0);
        }

        /** Same test as ModelPartCubeM.isBackFace, on transformed corners */
        private static boolean isBackFace(long[] xy, long[] zs, int c0, int c1, int c2) {
            float x0 = lowFloat(xy[c0]), y0 = highFloat(xy[c0]), z0 = lowFloat(zs[c0]);
            float ax = lowFloat(xy[c1]) - x0, ay = highFloat(xy[c1]) - y0, az = lowFloat(zs[c1]) - z0;
            float bx = lowFloat(xy[c2]) - x0, by = highFloat(xy[c2]) - y0, bz = lowFloat(zs[c2]) - z0;
            float nx = ay * bz - az * by;
            float ny = az * bx - ax * bz;
            float nz = ax * by - ay * bx;

            float behind = nx * x0 + ny * y0 + nz * z0;
            return behind > 0.0f && behind * behind >= EYE_MARGIN_SQ * (nx * nx + ny * ny + nz * nz);
        }

        private static float lowFloat(long bits) {
            return Float.intBitsToFloat((int) bits);
        }

        private static float highFloat(long bits) {
            return Float.intBitsToFloat((int) (bits >>> 32));
        }
    }

    /** Two 32-bit values as one little-endian 64-bit store: {@code low} first */
    private static long pack(int low, int high) {
        return (low & 0xFFFFFFFFL) | (long) high << 32;
    }

    private static int nextCornerStamp() {
        if (++cornerStamp == 0) {
            Arrays.fill(cornerStamps, 0);
            cornerStamp = 1;
        }
        return cornerStamp;
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
