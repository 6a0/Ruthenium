package net.ruthenium.render.blockentity;

import net.minecraft.client.renderer.rendertype.RenderSetup;
import net.minecraft.client.renderer.rendertype.RenderType;
import net.ruthenium.render.entity.TransparentFaces;
import org.jspecify.annotations.Nullable;
import org.lwjgl.system.MemoryUtil;

import java.nio.ByteBuffer;

/**
 * Whether a model-space mesh drawn without face culling looks the same with back faces culled: every quad belongs to a
 * closed, convex box wound outward (six consecutive quads over eight shared corners, as {@code ModelPart.Cube} writes a
 * cube with all faces) and every texel its faces sample is opaque in the alpha cutout. Then each pixel of a back face
 * lies under a front face of its own box, nearer to the eye and never discarded, which hides it, as long as the eye
 * is outside the box (the caller checks that per instance, with {@link #radius}) and the instance's transform keeps
 * the winding (positive determinant). The only pixels that can differ are those whose center lies, within float
 * rounding, on a box's edge (the texel past the edge, or a depth tie between the two faces there).
 */
final class ClosedBoxes {
    /** Model-space center and radius of the mesh's bounding sphere, set by {@link #check} */
    static float centerX, centerY, centerZ, radius;

    private ClosedBoxes() {
    }

    /**
     * @param vertices the mesh in {@code DefaultVertexFormat.ENTITY} (position at 0, UV0 at 16), {@code quads} quads
     * @return true if the mesh is made of closed opaque boxes only ({@link #centerX} and the others then hold its bounds)
     */
    static boolean check(ByteBuffer vertices, int quads, int stride, RenderType renderType) {
        if (quads == 0 || quads % 6 != 0) {
            return false;
        }
        TransparentFaces.TextureAlpha alpha = textureAlpha(renderType);
        if (alpha == null) {
            return false;
        }
        long base = MemoryUtil.memAddress(vertices);
        float[] corners = new float[24];
        int[] uses = new int[8];
        int[] index = new int[24];
        float minX = Float.POSITIVE_INFINITY, minY = Float.POSITIVE_INFINITY, minZ = Float.POSITIVE_INFINITY;
        float maxX = Float.NEGATIVE_INFINITY, maxY = Float.NEGATIVE_INFINITY, maxZ = Float.NEGATIVE_INFINITY;
        for (int box = 0; box < quads / 6; box++) {
            // The box's distinct corners, by exact value (a corner is the same vertex transformed the same way)
            int distinct = 0;
            java.util.Arrays.fill(uses, 0);
            for (int v = 0; v < 24; v++) {
                long p = base + (long) (box * 24 + v) * stride;
                float x = MemoryUtil.memGetFloat(p), y = MemoryUtil.memGetFloat(p + 4), z = MemoryUtil.memGetFloat(p + 8);
                if (!Float.isFinite(x) || !Float.isFinite(y) || !Float.isFinite(z)) {
                    return false;
                }
                int found = -1;
                for (int c = 0; c < distinct; c++) {
                    if (corners[c * 3] == x && corners[c * 3 + 1] == y && corners[c * 3 + 2] == z) {
                        found = c;
                        break;
                    }
                }
                if (found < 0) {
                    if (distinct == 8) {
                        return false;
                    }
                    found = distinct++;
                    corners[found * 3] = x;
                    corners[found * 3 + 1] = y;
                    corners[found * 3 + 2] = z;
                }
                uses[found]++;
                index[v] = found;
                minX = Math.min(minX, x);
                minY = Math.min(minY, y);
                minZ = Math.min(minZ, z);
                maxX = Math.max(maxX, x);
                maxY = Math.max(maxY, y);
                maxZ = Math.max(maxZ, z);
            }
            if (distinct != 8) {
                return false;
            }
            double cx = 0, cy = 0, cz = 0;
            for (int c = 0; c < 8; c++) {
                if (uses[c] != 3) {
                    return false;
                }
                cx += corners[c * 3];
                cy += corners[c * 3 + 1];
                cz += corners[c * 3 + 2];
            }
            cx /= 8;
            cy /= 8;
            cz /= 8;
            double size = 0;
            for (int c = 0; c < 8; c++) {
                size = Math.max(size, Math.abs(corners[c * 3] - cx) + Math.abs(corners[c * 3 + 1] - cy) + Math.abs(corners[c * 3 + 2] - cz));
            }
            for (int q = 0; q < 6; q++) {
                int i0 = index[q * 4], i1 = index[q * 4 + 1], i2 = index[q * 4 + 2], i3 = index[q * 4 + 3];
                if (i0 == i1 || i0 == i2 || i0 == i3 || i1 == i2 || i1 == i3 || i2 == i3) {
                    return false;
                }
                // Outward winding (counter-clockwise seen from outside, as vanilla's culling expects), on both triangles
                double[] n = cross(corners, i0, i1, i2);
                double[] n2 = cross(corners, i2, i3, i0);
                double length = Math.sqrt(n[0] * n[0] + n[1] * n[1] + n[2] * n[2]);
                if (length == 0 || n[0] * n2[0] + n[1] * n2[1] + n[2] * n2[2] <= 0
                        || n[0] * (corners[i0 * 3] - cx) + n[1] * (corners[i0 * 3 + 1] - cy) + n[2] * (corners[i0 * 3 + 2] - cz) <= 0) {
                    return false;
                }
                // Convex: no corner in front of the face's plane
                double tolerance = 1.0e-4 * length * size;
                for (int c = 0; c < 8; c++) {
                    double d = n[0] * (corners[c * 3] - corners[i0 * 3]) + n[1] * (corners[c * 3 + 1] - corners[i0 * 3 + 1])
                            + n[2] * (corners[c * 3 + 2] - corners[i0 * 3 + 2]);
                    if (d > tolerance) {
                        return false;
                    }
                }
                // Every texel the face samples is kept by the cutout
                float u0 = Float.POSITIVE_INFINITY, u1 = Float.NEGATIVE_INFINITY, v0 = Float.POSITIVE_INFINITY, v1 = Float.NEGATIVE_INFINITY;
                for (int v = 0; v < 4; v++) {
                    long p = base + (long) (box * 24 + q * 4 + v) * stride + 16;
                    float u = MemoryUtil.memGetFloat(p), w = MemoryUtil.memGetFloat(p + 4);
                    u0 = Math.min(u0, u);
                    u1 = Math.max(u1, u);
                    v0 = Math.min(v0, w);
                    v1 = Math.max(v1, w);
                }
                if (!alpha.isOpaqueNearest(u0, u1, v0, v1)) {
                    return false;
                }
            }
        }
        centerX = (minX + maxX) * 0.5f;
        centerY = (minY + maxY) * 0.5f;
        centerZ = (minZ + maxZ) * 0.5f;
        float r = 0;
        for (int v = 0; v < quads * 4; v++) {
            long p = base + (long) v * stride;
            float dx = MemoryUtil.memGetFloat(p) - centerX, dy = MemoryUtil.memGetFloat(p + 4) - centerY, dz = MemoryUtil.memGetFloat(p + 8) - centerZ;
            r = Math.max(r, (float) Math.sqrt(dx * dx + dy * dy + dz * dz));
        }
        radius = r * 1.001f + 1.0e-3f;
        return true;
    }

    private static double[] cross(float[] corners, int a, int b, int c) {
        double e1x = corners[b * 3] - corners[a * 3], e1y = corners[b * 3 + 1] - corners[a * 3 + 1], e1z = corners[b * 3 + 2] - corners[a * 3 + 2];
        double e2x = corners[c * 3] - corners[b * 3], e2y = corners[c * 3 + 1] - corners[b * 3 + 1], e2z = corners[c * 3 + 2] - corners[b * 3 + 2];
        return new double[]{e1y * e2z - e1z * e2y, e1z * e2x - e1x * e2z, e1x * e2y - e1y * e2x};
    }

    /** The cutout's texture alpha, if the render type samples a plain texture with its own (nearest) sampler */
    private static TransparentFaces.@Nullable TextureAlpha textureAlpha(RenderType renderType) {
        RenderSetup.TextureBinding binding = renderType.state.textures.get("Sampler0");
        if (binding == null || binding.sampler().get() != null) {
            return null;
        }
        return TransparentFaces.of(renderType);
    }
}
