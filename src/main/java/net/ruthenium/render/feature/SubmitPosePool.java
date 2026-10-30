package net.ruthenium.render.feature;

import com.mojang.blaze3d.systems.RenderSystem;
import com.mojang.blaze3d.vertex.PoseStack;
import net.ruthenium.Initializer;

import java.util.Arrays;

/**
 * The poses that {@code SubmitNodeCollection} copies for its submits ({@code poseStack.last().copy()}, a {@code Pose},
 * a {@code Matrix4f} and a {@code Matrix3f} per submit), taken from a pool that is reused from the next frame on.
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

    private SubmitPosePool() {
    }

    /** A new frame: the poses handed out before are free again */
    public static void beginFrame() {
        used = 0;
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
}
