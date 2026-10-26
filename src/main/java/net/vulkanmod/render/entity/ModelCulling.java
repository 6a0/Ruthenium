package net.vulkanmod.render.entity;

import net.minecraft.client.renderer.rendertype.RenderType;

/**
 * Decides when model cubes may skip back faces on the CPU (see {@code ModelPartCubeM}).
 * <p>
 * Back faces of render types whose pipeline culls are discarded by the GPU anyway, so not writing them gives the
 * same image. This is only done while preparing the world's features: their positions are relative to the camera,
 * so the eye is at the origin, up to view bobbing (~0.1 blocks; nausea warps the projection linearly about the
 * eye). A face is skipped only when the eye is at least {@link #EYE_MARGIN} behind its plane. GUI, hand and other
 * passes are left alone. Only used from the render thread.
 */
public final class ModelCulling {
    public static final float EYE_MARGIN = 0.5f;

    private static boolean worldPass;
    private static boolean cullBackFaces;

    private ModelCulling() {
    }

    public static void setWorldPass(boolean active) {
        worldPass = active;
        cullBackFaces = false;
    }

    public static void beginModel(RenderType renderType) {
        cullBackFaces = worldPass && renderType.pipeline().isCull();
    }

    public static void endModel() {
        cullBackFaces = false;
    }

    public static boolean cullBackFaces() {
        return cullBackFaces;
    }
}
