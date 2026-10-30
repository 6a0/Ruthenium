package net.ruthenium.render.feature;

import com.mojang.blaze3d.systems.RenderSystem;
import com.mojang.blaze3d.vertex.PoseStack;
import net.ruthenium.Initializer;
import org.joml.Matrix4f;
import org.joml.Matrix4fc;

import java.util.Arrays;

/**
 * The poses that {@code SubmitNodeCollection} copies for its submits ({@code poseStack.last().copy()}, a {@code Pose},
 * a {@code Matrix4f} and a {@code Matrix3f} per submit), and the matrices it copies for text, name tag, shadow, leash and
 * moving block submits ({@code new Matrix4f(pose)}), taken from pools that are reused from the next frame on.
 * <p>
 * Vanilla's submits live until the frame's features are prepared and executed: a submit node storage is drained into
 * the feature dispatcher's prepared frame, which is closed at the end of the pass that rendered it, all within one
 * {@code Minecraft.renderFrame}. So a pooled pose is handed out at most once per frame, gets the same values as
 * {@code copy()} ({@code Pose.set}: both matrices with their properties and the trusted normals flag), and is only
 * written again in a later frame. Render thread only; elsewhere, and past {@link #MAX_POSES} in a frame, poses are
 * copied as usual.
 */
public final class SubmitPosePool {
    private static final int MAX_POSES = 1 << 16;

    private static PoseStack.Pose[] poses = new PoseStack.Pose[1024];
    private static int used;
    private static Matrix4f[] matrices = new Matrix4f[256];
    private static int usedMatrices;

    private SubmitPosePool() {
    }

    /** A new frame: the poses handed out before are free again */
    public static void beginFrame() {
        used = 0;
        usedMatrices = 0;
    }

    /** As {@code pose.copy()} */
    public static PoseStack.Pose copy(PoseStack.Pose pose) {
        if (!Initializer.CONFIG.pooledSubmitPoses || !RenderSystem.isOnRenderThread()) {
            return pose.copy();
        }
        int index = used;
        PoseStack.Pose[] poses = SubmitPosePool.poses;
        if (index == poses.length) {
            if (index >= MAX_POSES) {
                return pose.copy();
            }
            poses = SubmitPosePool.poses = Arrays.copyOf(poses, index * 2);
        }
        PoseStack.Pose copy = poses[index];
        if (copy == null) {
            copy = poses[index] = new PoseStack.Pose();
        }
        used = index + 1;
        copy.set(pose);
        return copy;
    }

    /** As {@code new Matrix4f(matrix)} ({@code Matrix4f.set} copies the same elements and properties) */
    public static Matrix4f copy(Matrix4fc matrix) {
        if (!Initializer.CONFIG.pooledSubmitPoses || !RenderSystem.isOnRenderThread()) {
            return new Matrix4f(matrix);
        }
        int index = usedMatrices;
        Matrix4f[] matrices = SubmitPosePool.matrices;
        if (index == matrices.length) {
            if (index >= MAX_POSES) {
                return new Matrix4f(matrix);
            }
            matrices = SubmitPosePool.matrices = Arrays.copyOf(matrices, index * 2);
        }
        Matrix4f copy = matrices[index];
        if (copy == null) {
            copy = matrices[index] = new Matrix4f();
        }
        usedMatrices = index + 1;
        return copy.set(matrix);
    }
}
