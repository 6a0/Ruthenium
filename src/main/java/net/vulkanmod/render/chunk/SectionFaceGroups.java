package net.vulkanmod.render.chunk;

import com.mojang.blaze3d.vertex.DefaultVertexFormat;
import com.mojang.blaze3d.vertex.MeshData;
import com.mojang.renderpearl.api.pipeline.PrimitiveTopology;
import net.minecraft.client.renderer.DynamicGpuData;
import org.jspecify.annotations.Nullable;
import org.lwjgl.system.MemoryUtil;

import java.nio.ByteBuffer;
import java.util.List;

/**
 * Quads of a section layer grouped by the axis-aligned plane they face, so a frame can skip the groups that face
 * away from the camera.
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

    /** Draws a group when the eye is within this distance behind its frontmost plane, against rounding */
    private static final float MARGIN = 1.0e-3f;
    private static final int QUAD_BYTES = 4 * 28;

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
     * Reorders the quads of a layer mesh (block vertex format, sequential quads) by facing.
     *
     * @return the groups, or null if the mesh isn't one this handles or has nothing to skip
     */
    public static @Nullable SectionFaceGroups sort(MeshData mesh) {
        MeshData.DrawState state = mesh.drawState();
        if (mesh.indexBuffer() != null || state.format() != DefaultVertexFormat.BLOCK
                || state.primitiveTopology() != PrimitiveTopology.QUADS) {
            return null;
        }

        ByteBuffer vertices = mesh.vertexBuffer();
        int quadCount = state.vertexCount() / 4;
        if (quadCount == 0 || vertices.remaining() < quadCount * QUAD_BYTES) {
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

        for (int q = 0; q < quadCount; q++) {
            long p = base + (long) q * QUAD_BYTES;
            float x0 = MemoryUtil.memGetFloat(p), y0 = MemoryUtil.memGetFloat(p + 4), z0 = MemoryUtil.memGetFloat(p + 8);
            float x1 = MemoryUtil.memGetFloat(p + 28), y1 = MemoryUtil.memGetFloat(p + 32), z1 = MemoryUtil.memGetFloat(p + 36);
            float x2 = MemoryUtil.memGetFloat(p + 56), y2 = MemoryUtil.memGetFloat(p + 60), z2 = MemoryUtil.memGetFloat(p + 64);
            float x3 = MemoryUtil.memGetFloat(p + 84), y3 = MemoryUtil.memGetFloat(p + 88), z3 = MemoryUtil.memGetFloat(p + 92);

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

        long scratch = MemoryUtil.nmemAlloc((long) quadCount * QUAD_BYTES);
        try {
            int[] next = new int[GROUPS];
            System.arraycopy(starts, 0, next, 0, GROUPS);
            for (int q = 0; q < quadCount; q++) {
                int dst = next[groupOf[q]]++;
                MemoryUtil.memCopy(base + (long) q * QUAD_BYTES, scratch + (long) dst * QUAD_BYTES, QUAD_BYTES);
            }
            MemoryUtil.memCopy(scratch, base, (long) quadCount * QUAD_BYTES);
        } finally {
            MemoryUtil.nmemFree(scratch);
        }
        return groups;
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
