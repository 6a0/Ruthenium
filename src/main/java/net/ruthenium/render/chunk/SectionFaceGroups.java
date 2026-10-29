package net.ruthenium.render.chunk;

import com.mojang.blaze3d.vertex.DefaultVertexFormat;
import com.mojang.blaze3d.vertex.MeshData;
import com.mojang.renderpearl.api.pipeline.PrimitiveTopology;
import com.mojang.renderpearl.api.vertex.VertexFormat;
import net.minecraft.client.renderer.DynamicGpuData;
import org.jspecify.annotations.Nullable;
import org.lwjgl.system.MemoryUtil;

import java.nio.ByteBuffer;
import java.util.List;

/**
 * Quads of a section layer (or of a retained block entity batch) grouped by the axis-aligned plane they face, so a
 * frame can skip the groups that face away from the camera.
 * <p>
 * At compile time the layer's quads are reordered (stably) into the groups below, by the winding normal of each
 * quad that lies in an axis-aligned plane. Terrain pipelines cull back faces, and with a perspective projection a
 * quad is back facing exactly when the eye is behind its plane, so a group whose planes are all in front of the eye
 * would be discarded by the GPU anyway. Skipping it changes no pixels; the order of the quads that are drawn only
 * changes between different planes or facings, where it doesn't matter for opaque and cutout geometry.
 */
public final class SectionFaceGroups {
    /** Quads that are not in an axis-aligned plane; always drawn */
    public static final int UNASSIGNED = 0;
    public static final int EAST = 1;
    public static final int WEST = 2;
    public static final int UP = 3;
    public static final int DOWN = 4;
    public static final int SOUTH = 5;
    public static final int NORTH = 6;
    private static final int GROUPS = 7;
    /** Runs of {@link #visibleRuns} at most: drawn and skipped groups alternate */
    public static final int MAX_RUNS = (GROUPS + 1) / 2;
    /** Runs of {@link #visiblePlaneRuns} at most: one per group */
    public static final int MAX_PLANE_RUNS = GROUPS;
    /** Largest tilt (other normal components relative to the main one) of a quad counted as axis-aligned by plane */
    private static final float TILT = 1.0e-5f;
    /** Largest spread of a near axis-aligned quad's plane coordinates */
    private static final float SPREAD = 1.0e-4f;

    /**
     * Draws a group when the eye is within this distance behind its frontmost plane. The tests use the camera
     * position, but view bobbing is multiplied into the projection and moves the eye up to ~0.11 blocks from it
     * ({@code GameRenderer.bobView}: a translation of {@code bob * 0.5} and {@code bob}, bob <= 0.1).
     */
    private static final float MARGIN = 0.25f;

    /** First quad of each group, and the quad count at the end */
    private final int[] starts = new int[GROUPS + 1];
    /**
     * Per facing group, the plane coordinate (section-relative) from which on some of its quads face the eye: the
     * lowest plane of groups facing the positive direction, the highest of those facing the negative direction
     */
    private final float[] planes = new float[GROUPS];

    /**
     * Per plane sorted groups ({@link #sortByPlanes}): the first index into {@link #planeTests} of each group (and the
     * end), each plane's test value (margin applied) and the end of its quads; null otherwise
     */
    private int @Nullable [] groupPlanes;
    private float @Nullable [] planeTests;
    private int @Nullable [] planeEnds;

    private SectionFaceGroups() {
    }

    /**
     * Like {@link #sort}, but also orders each facing group's quads by plane, nearest to the side the group faces
     * first, so a frame can draw only the quads whose plane the eye is in front of ({@link #visiblePlaneRuns}) instead
     * of the whole group when any of them can face the eye. Quads whose normal is within a tilt of 1e-5 of an axis and
     * whose vertices are within 1e-4 blocks of a plane count as in that plane (rotated models: a rotation by a multiple
     * of 90 degrees leaves coordinates like 6e-17 instead of 0); their plane is their outermost vertex coordinate
     * towards the back, so the test stays conservative (the margin covers the tilt over thousands of blocks). The
     * order only changes between quads of different planes or facings: equal plane quads keep their order.
     * <p>
     * The mesh is left as it is: the reordered vertices go to {@code target} (at least as many bytes as the mesh's vertex buffer has
     * remaining from its position), which is then uploaded instead of the mesh. Nothing is written when this returns null.
     *
     * @return the groups, or null if the mesh isn't one this handles or has no quad in an axis-aligned plane
     */
    public static @Nullable SectionFaceGroups sortByPlanes(MeshData mesh, ByteBuffer target) {
        MeshData.DrawState state = mesh.drawState();
        VertexFormat format = state.format();
        boolean floatPosition = format == DefaultVertexFormat.BLOCK || format == DefaultVertexFormat.ENTITY
                || format == DefaultVertexFormat.POSITION_TEX_LIGHTMAP_COLOR;
        if (mesh.indexBuffer() != null || !floatPosition || state.primitiveTopology() != PrimitiveTopology.QUADS) {
            return null;
        }
        int stride = format.getVertexSize();
        int quadBytes = 4 * stride;
        ByteBuffer vertices = mesh.vertexBuffer();
        int quadCount = state.vertexCount() / 4;
        if (quadCount == 0 || quadCount >= 1 << 20 || vertices.remaining() < quadCount * quadBytes) {
            return null;
        }

        long base = MemoryUtil.memAddress(vertices);
        long[] keys = new long[quadCount];
        float[] quadPlanes = new float[quadCount];
        int[] counts = new int[GROUPS];
        float[] position = new float[12];
        for (int q = 0; q < quadCount; q++) {
            readPositions(base + (long) q * quadBytes, stride, false, position);
            float ax = position[6] - position[0], ay = position[7] - position[1], az = position[8] - position[2];
            float bx = position[9] - position[3], by = position[10] - position[4], bz = position[11] - position[5];
            float nx = ay * bz - az * by;
            float ny = az * bx - ax * bz;
            float nz = ax * by - ay * bx;
            float anx = Math.abs(nx), any = Math.abs(ny), anz = Math.abs(nz);

            int group = UNASSIGNED;
            int axis = -1;
            if (anx > 0.0f && any <= anx * TILT && anz <= anx * TILT) {
                axis = 0;
                group = nx > 0.0f ? EAST : WEST;
            } else if (any > 0.0f && anx <= any * TILT && anz <= any * TILT) {
                axis = 1;
                group = ny > 0.0f ? UP : DOWN;
            } else if (anz > 0.0f && anx <= anz * TILT && any <= anz * TILT) {
                axis = 2;
                group = nz > 0.0f ? SOUTH : NORTH;
            }
            float plane = 0.0f;
            if (axis >= 0) {
                float lo = Math.min(Math.min(position[axis], position[3 + axis]), Math.min(position[6 + axis], position[9 + axis]));
                float hi = Math.max(Math.max(position[axis], position[3 + axis]), Math.max(position[6 + axis], position[9 + axis]));
                if (hi - lo <= SPREAD) {
                    // Groups facing the positive direction are drawn from their lowest plane on, the others up to their highest
                    plane = (group & 1) == 1 ? lo : hi;
                } else {
                    group = UNASSIGNED;
                }
            }
            quadPlanes[q] = plane;
            counts[group]++;
            // Nearest to the faced side first: ascending planes for positive facings, descending for negative ones
            int order = group == UNASSIGNED ? 0 : sortable(plane);
            if (group != UNASSIGNED && (group & 1) == 0) {
                order = ~order;
            }
            keys[q] = (long) group << 52 | ((order ^ Integer.MIN_VALUE) & 0xFFFFFFFFL) << 20 | q;
        }
        if (counts[UNASSIGNED] == quadCount) {
            return null;
        }
        keys = sortKeys(keys);

        SectionFaceGroups groups = new SectionFaceGroups();
        int[] starts = groups.starts;
        for (int g = 0; g < GROUPS; g++) {
            starts[g + 1] = starts[g] + counts[g];
        }
        int[] groupPlanes = new int[GROUPS + 1];
        float[] tests = new float[quadCount];
        int[] ends = new int[quadCount];
        int planeCount = 0;
        float[] planes = groups.planes;
        for (int g = 1; g < GROUPS; g++) {
            planes[g] = (g & 1) == 1 ? Float.POSITIVE_INFINITY : Float.NEGATIVE_INFINITY;
        }
        int g = 0;
        for (int i = 0; i < quadCount; i++) {
            int q = (int) (keys[i] & 0xFFFFF);
            int group = (int) (keys[i] >>> 52);
            while (g < group) {
                groupPlanes[++g] = planeCount;
            }
            if (group == UNASSIGNED) {
                continue;
            }
            float plane = quadPlanes[q];
            planes[group] = (group & 1) == 1 ? Math.min(planes[group], plane) : Math.max(planes[group], plane);
            float test = (group & 1) == 1 ? plane - MARGIN : plane + MARGIN;
            if (planeCount > groupPlanes[group] && Float.floatToRawIntBits(tests[planeCount - 1]) == Float.floatToRawIntBits(test)) {
                ends[planeCount - 1] = i + 1;
            } else {
                tests[planeCount] = test;
                ends[planeCount] = i + 1;
                planeCount++;
            }
        }
        while (g < GROUPS) {
            groupPlanes[++g] = planeCount;
        }
        groups.groupPlanes = groupPlanes;
        groups.planeTests = java.util.Arrays.copyOf(tests, planeCount);
        groups.planeEnds = java.util.Arrays.copyOf(ends, planeCount);

        long dst = MemoryUtil.memAddress(target);
        for (int i = 0; i < quadCount; i++) {
            int q = (int) (keys[i] & 0xFFFFF);
            MemoryUtil.memCopy(base + (long) q * quadBytes, dst + (long) i * quadBytes, quadBytes);
        }
        // Any bytes after the last whole quad stay where they were
        long sortedBytes = (long) quadCount * quadBytes;
        MemoryUtil.memCopy(base + sortedBytes, dst + sortedBytes, vertices.remaining() - sortedBytes);
        return groups;
    }

    /**
     * The keys of {@link #sortByPlanes} in ascending order, as {@code Arrays.sort} puts them: a stable LSD radix sort of their
     * group and plane bits (20..55) in three 12 bit passes, skipping a pass whose digit is the same in every key. The keys come
     * in ascending quad index order (bits 0..19, unique), and stable passes keep that order among keys of equal group and plane,
     * so the result is the order of the whole keys.
     *
     * @return the sorted keys: the given array or a new one
     */
    private static long[] sortKeys(long[] keys) {
        int n = keys.length;
        int[] counts = new int[3 << 12];
        for (int i = 0; i < n; i++) {
            long k = keys[i];
            counts[(int) (k >>> 20) & 0xFFF]++;
            counts[(1 << 12) + ((int) (k >>> 32) & 0xFFF)]++;
            counts[(2 << 12) + ((int) (k >>> 44) & 0xFFF)]++;
        }
        long[] a = keys;
        long[] b = null;
        for (int pass = 0; pass < 3; pass++) {
            int shift = 20 + 12 * pass;
            int offset = pass << 12;
            if (counts[offset + ((int) (a[0] >>> shift) & 0xFFF)] == n) {
                continue;
            }
            if (b == null) {
                b = new long[n];
            }
            int sum = 0;
            for (int d = offset; d < offset + (1 << 12); d++) {
                int c = counts[d];
                counts[d] = sum;
                sum += c;
            }
            for (int i = 0; i < n; i++) {
                long k = a[i];
                b[counts[offset + ((int) (k >>> shift) & 0xFFF)]++] = k;
            }
            long[] t = a;
            a = b;
            b = t;
        }
        return a;
    }

    /** An int whose signed order is the float's order */
    private static int sortable(float f) {
        int bits = Float.floatToIntBits(f == 0.0f ? 0.0f : f);
        return bits ^ (bits >> 31 & 0x7FFFFFFF);
    }

    /**
     * The runs of quads to draw for an eye position, of groups sorted by {@link #sortByPlanes}: of each facing group
     * the quads whose plane the eye can be in front of, merging adjacent runs.
     *
     * @param eyeX eye position relative to the mesh's origin
     * @param runs receives the first quad and quad count of each run ({@link #MAX_PLANE_RUNS} pairs at most)
     * @return the number of runs
     */
    public int visiblePlaneRuns(float eyeX, float eyeY, float eyeZ, int[] runs) {
        int[] groupPlanes = this.groupPlanes;
        float[] tests = this.planeTests;
        int[] ends = this.planeEnds;
        int count = 0;
        int runStart = 0;
        int runEnd = this.starts[1];
        for (int g = 1; g < GROUPS; g++) {
            int start = this.starts[g];
            int end = start;
            int first = groupPlanes[g], last = groupPlanes[g + 1];
            float eye = g <= WEST ? eyeX : g <= DOWN ? eyeY : eyeZ;
            if ((g & 1) == 1) {
                for (int p = first; p < last && eye > tests[p]; p++) {
                    end = ends[p];
                }
            } else {
                for (int p = first; p < last && eye < tests[p]; p++) {
                    end = ends[p];
                }
            }
            if (end == start) {
                continue;
            }
            if (start == runEnd) {
                runEnd = end;
            } else {
                if (runEnd > runStart) {
                    runs[count * 2] = runStart;
                    runs[count * 2 + 1] = runEnd - runStart;
                    count++;
                }
                runStart = start;
                runEnd = end;
            }
        }
        if (runEnd > runStart) {
            runs[count * 2] = runStart;
            runs[count * 2 + 1] = runEnd - runStart;
            count++;
        }
        return count;
    }

    /**
     * Reorders the quads of a mesh (sequential quads in vanilla's block, entity or world text vertex format, or
     * {@link CompressedTerrain#FORMAT}) by facing.
     *
     * @return the groups, or null if the mesh isn't one this handles or has nothing to skip
     */
    public static @Nullable SectionFaceGroups sort(MeshData mesh) {
        MeshData.DrawState state = mesh.drawState();
        VertexFormat format = state.format();
        boolean compressed = format == CompressedTerrain.FORMAT;
        // These formats start with a float position
        boolean floatPosition = format == DefaultVertexFormat.BLOCK || format == DefaultVertexFormat.ENTITY
                || format == DefaultVertexFormat.POSITION_TEX_LIGHTMAP_COLOR;
        if (mesh.indexBuffer() != null || !floatPosition && !compressed || state.primitiveTopology() != PrimitiveTopology.QUADS) {
            return null;
        }
        int stride = format.getVertexSize();
        int quadBytes = 4 * stride;

        ByteBuffer vertices = mesh.vertexBuffer();
        int quadCount = state.vertexCount() / 4;
        if (quadCount == 0 || vertices.remaining() < quadCount * quadBytes) {
            return null;
        }

        long base = MemoryUtil.memAddress(vertices);
        byte[] groupOf = new byte[quadCount];
        int[] counts = new int[GROUPS];
        SectionFaceGroups groups = new SectionFaceGroups();
        float[] planes = groups.planes;
        for (int g = 1; g < GROUPS; g++) {
            planes[g] = (g & 1) == 1 ? Float.POSITIVE_INFINITY : Float.NEGATIVE_INFINITY;
        }

        float[] position = new float[12];
        for (int q = 0; q < quadCount; q++) {
            readPositions(base + (long) q * quadBytes, stride, compressed, position);
            float x0 = position[0], y0 = position[1], z0 = position[2];
            float x1 = position[3], y1 = position[4], z1 = position[5];
            float x2 = position[6], y2 = position[7], z2 = position[8];
            float x3 = position[9], y3 = position[10], z3 = position[11];

            // Normal of the quad's winding from its diagonals (also right for a quad with a collapsed edge)
            float ax = x2 - x0, ay = y2 - y0, az = z2 - z0;
            float bx = x3 - x1, by = y3 - y1, bz = z3 - z1;
            float nx = ay * bz - az * by;
            float ny = az * bx - ax * bz;
            float nz = ax * by - ay * bx;

            int group = UNASSIGNED;
            float plane = 0.0f;
            if (x0 == x1 && x0 == x2 && x0 == x3 && nx != 0.0f) {
                group = nx > 0.0f ? EAST : WEST;
                plane = x0;
            } else if (y0 == y1 && y0 == y2 && y0 == y3 && ny != 0.0f) {
                group = ny > 0.0f ? UP : DOWN;
                plane = y0;
            } else if (z0 == z1 && z0 == z2 && z0 == z3 && nz != 0.0f) {
                group = nz > 0.0f ? SOUTH : NORTH;
                plane = z0;
            }

            if (group != UNASSIGNED) {
                planes[group] = (group & 1) == 1 ? Math.min(planes[group], plane) : Math.max(planes[group], plane);
            }
            groupOf[q] = (byte) group;
            counts[group]++;
        }

        int[] starts = groups.starts;
        for (int g = 0; g < GROUPS; g++) {
            starts[g + 1] = starts[g] + counts[g];
        }
        for (int g = 0; g < GROUPS; g++) {
            if (counts[g] == quadCount) {
                // A single group is already in order, and can only be skipped if it faces somewhere
                return g == UNASSIGNED ? null : groups;
            }
        }

        long scratch = MemoryUtil.nmemAlloc((long) quadCount * quadBytes);
        try {
            int[] next = new int[GROUPS];
            System.arraycopy(starts, 0, next, 0, GROUPS);
            for (int q = 0; q < quadCount; q++) {
                int dst = next[groupOf[q]]++;
                MemoryUtil.memCopy(base + (long) q * quadBytes, scratch + (long) dst * quadBytes, quadBytes);
            }
            MemoryUtil.memCopy(scratch, base, (long) quadCount * quadBytes);
        } finally {
            MemoryUtil.nmemFree(scratch);
        }
        return groups;
    }

    /** The positions of a quad's 4 vertices, as the vertex shader sees them */
    private static void readPositions(long quad, int stride, boolean compressed, float[] out) {
        for (int i = 0; i < 4; i++) {
            long vertex = quad + (long) i * stride;
            if (compressed) {
                long packed = MemoryUtil.memGetLong(vertex + CompressedTerrain.POSITION_OFFSET);
                out[i * 3] = CompressedTerrain.unpackCoordinate(packed, 0);
                out[i * 3 + 1] = CompressedTerrain.unpackCoordinate(packed, 1);
                out[i * 3 + 2] = CompressedTerrain.unpackCoordinate(packed, 2);
            } else {
                out[i * 3] = MemoryUtil.memGetFloat(vertex);
                out[i * 3 + 1] = MemoryUtil.memGetFloat(vertex + 4);
                out[i * 3 + 2] = MemoryUtil.memGetFloat(vertex + 8);
            }
        }
    }

    public int quadCount() {
        return this.starts[GROUPS];
    }

    /**
     * Adds the draws of the groups that can face the eye in place of the whole layer's draw, merging adjacent ones.
     *
     * @param draw the layer's draw (sequential quad indices, so a group is a vertex range)
     * @param eyeX eye position relative to the section origin
     */
    public void addVisibleDraws(List<DynamicGpuData.IndexedDraw> draws, DynamicGpuData.IndexedDraw draw,
                                float eyeX, float eyeY, float eyeZ) {
        int runStart = -1;
        for (int g = 0; g <= GROUPS; g++) {
            if (g < GROUPS && this.isDrawn(g, eyeX, eyeY, eyeZ)) {
                if (runStart < 0) {
                    runStart = this.starts[g];
                }
            } else if (runStart >= 0) {
                int runEnd = this.starts[g];
                if (runEnd > runStart) {
                    draws.add(new DynamicGpuData.IndexedDraw((runEnd - runStart) * 6, draw.instanceCount(), draw.firstIndex(),
                            draw.baseVertex() + runStart * 4, draw.baseInstance()));
                }
                runStart = -1;
            }
        }
    }

    /**
     * The runs of quads to draw for an eye position, merging adjacent groups.
     *
     * @param eyeX eye position relative to the mesh's origin
     * @param runs receives the first quad and quad count of each run ({@link #MAX_RUNS} pairs at most)
     * @return the number of runs
     */
    public int visibleRuns(float eyeX, float eyeY, float eyeZ, int[] runs) {
        int count = 0;
        int runStart = -1;
        for (int g = 0; g <= GROUPS; g++) {
            if (g < GROUPS && this.isDrawn(g, eyeX, eyeY, eyeZ)) {
                if (runStart < 0) {
                    runStart = this.starts[g];
                }
            } else if (runStart >= 0) {
                int runEnd = this.starts[g];
                if (runEnd > runStart) {
                    runs[count * 2] = runStart;
                    runs[count * 2 + 1] = runEnd - runStart;
                    count++;
                }
                runStart = -1;
            }
        }
        return count;
    }

    /**
     * Ints {@link #packInto} writes: the group starts (and the quad count at the end), the facing groups' plane bits
     * with the margin applied, and the mask of groups that are always drawn (unassigned or empty)
     */
    static final int PACKED_INTS = GROUPS + 1 + GROUPS - 1 + 1;
    private static final int PACKED_PLANES = GROUPS + 1 - 1;
    private static final int PACKED_ALWAYS = GROUPS + 1 + GROUPS - 1;
    /** Per mask of drawn groups: the first and end group of each run of drawn groups */
    private static final int[][] RUNS_BY_MASK = new int[1 << GROUPS][];

    static {
        for (int mask = 0; mask < RUNS_BY_MASK.length; mask++) {
            int[] runs = new int[MAX_RUNS * 2];
            int count = 0;
            int runStart = -1;
            for (int g = 0; g <= GROUPS; g++) {
                if (g < GROUPS && (mask & 1 << g) != 0) {
                    if (runStart < 0) {
                        runStart = g;
                    }
                } else if (runStart >= 0) {
                    runs[count++] = runStart;
                    runs[count++] = g;
                    runStart = -1;
                }
            }
            RUNS_BY_MASK[mask] = java.util.Arrays.copyOf(runs, count);
        }
    }

    /** Writes the groups into {@code data} at {@code offset}, for {@link #addVisibleCommands(int[], int, TerrainDrawList.Group, int, int, int, float, float, float)} */
    void packInto(int[] data, int offset) {
        System.arraycopy(this.starts, 0, data, offset, GROUPS + 1);
        int always = 1 << UNASSIGNED;
        for (int g = 1; g < GROUPS; g++) {
            // The same float operation as faces()
            float plane = (g & 1) == 1 ? this.planes[g] - MARGIN : this.planes[g] + MARGIN;
            data[offset + PACKED_PLANES + g] = Float.floatToRawIntBits(plane);
            if (this.starts[g] == this.starts[g + 1]) {
                always |= 1 << g;
            }
        }
        data[offset + PACKED_ALWAYS] = always;
    }

    /**
     * {@link #addVisibleCommands(TerrainDrawList.Group, int, int, int, float, float, float)} from packed groups, so
     * the draw list reads one array per section instead of the groups' objects, and finds the runs to draw from a
     * mask of the drawn groups instead of a loop with a branch per group
     */
    static void addVisibleCommands(int[] data, int offset, TerrainDrawList.Group group, int firstIndex, int baseVertex,
                                   int baseInstance, float eyeX, float eyeY, float eyeZ) {
        int p = offset + PACKED_PLANES;
        int mask = data[offset + PACKED_ALWAYS]
                | (eyeX > Float.intBitsToFloat(data[p + EAST]) ? 1 << EAST : 0)
                | (eyeX < Float.intBitsToFloat(data[p + WEST]) ? 1 << WEST : 0)
                | (eyeY > Float.intBitsToFloat(data[p + UP]) ? 1 << UP : 0)
                | (eyeY < Float.intBitsToFloat(data[p + DOWN]) ? 1 << DOWN : 0)
                | (eyeZ > Float.intBitsToFloat(data[p + SOUTH]) ? 1 << SOUTH : 0)
                | (eyeZ < Float.intBitsToFloat(data[p + NORTH]) ? 1 << NORTH : 0);
        int[] runs = RUNS_BY_MASK[mask];
        for (int r = 0; r < runs.length; r += 2) {
            int start = data[offset + runs[r]];
            int end = data[offset + runs[r + 1]];
            if (end > start) {
                group.add((end - start) * 6, firstIndex, baseVertex + start * 4, baseInstance);
            }
        }
    }

    /** Like {@link #addVisibleDraws}, for {@link TerrainDrawList}'s commands */
    void addVisibleCommands(TerrainDrawList.Group group, int firstIndex, int baseVertex, int baseInstance,
                            float eyeX, float eyeY, float eyeZ) {
        int runStart = -1;
        for (int g = 0; g <= GROUPS; g++) {
            if (g < GROUPS && this.isDrawn(g, eyeX, eyeY, eyeZ)) {
                if (runStart < 0) {
                    runStart = this.starts[g];
                }
            } else if (runStart >= 0) {
                int runEnd = this.starts[g];
                if (runEnd > runStart) {
                    group.add((runEnd - runStart) * 6, firstIndex, baseVertex + runStart * 4, baseInstance);
                }
                runStart = -1;
            }
        }
    }

    /** Empty groups count as drawn, so they don't split runs */
    private boolean isDrawn(int group, float eyeX, float eyeY, float eyeZ) {
        return this.starts[group] == this.starts[group + 1] || this.faces(group, eyeX, eyeY, eyeZ);
    }

    private boolean faces(int group, float eyeX, float eyeY, float eyeZ) {
        float plane = this.planes[group];
        return switch (group) {
            case EAST -> eyeX > plane - MARGIN;
            case WEST -> eyeX < plane + MARGIN;
            case UP -> eyeY > plane - MARGIN;
            case DOWN -> eyeY < plane + MARGIN;
            case SOUTH -> eyeZ > plane - MARGIN;
            case NORTH -> eyeZ < plane + MARGIN;
            default -> true;
        };
    }
}
