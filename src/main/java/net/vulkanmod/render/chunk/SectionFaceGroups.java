package net.vulkanmod.render.chunk;

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

    private SectionFaceGroups() {
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
