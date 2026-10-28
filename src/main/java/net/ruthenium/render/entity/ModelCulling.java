package net.ruthenium.render.entity;

import com.mojang.renderpearl.api.pipeline.ColorTargetState;
import com.mojang.renderpearl.api.pipeline.RenderPipeline;
import net.minecraft.client.renderer.RenderPipelines;
import net.minecraft.client.renderer.oit.OitPipelineSet;
import net.minecraft.client.renderer.rendertype.RenderType;
import net.minecraft.client.Minecraft;
import net.ruthenium.Initializer;
import org.jspecify.annotations.Nullable;
import org.joml.Matrix4f;

/**
 * Decides when model cubes may skip back faces on the CPU (see {@code ModelPartCubeM}).
 * <p>
 * Back faces of render types whose pipeline culls are discarded by the GPU anyway, so not writing them gives the
 * same image. This is only done while preparing the world's features: their positions are relative to the camera,
 * so the eye is at the origin, up to view bobbing (~0.1 blocks; nausea warps the projection linearly about the
 * eye). A face is skipped only when the eye is at least {@link #EYE_MARGIN} behind its plane. GUI, hand and other
 * passes are left alone. Only used from the render thread.
 * <p>
 * The same holds for sign text, which is submitted while the world's block entities are submitted: all glyphs of
 * one side lie in one plane, so a whole side can be skipped ({@link #isSignTextHidden}).
 */
public final class ModelCulling {
    public static final float EYE_MARGIN = 0.5f;
    /** Extra margin for text: glyph effects and outlines sit slightly off the text plane */
    private static final float TEXT_EYE_MARGIN = EYE_MARGIN + 0.05f;

    private static boolean worldPass;
    private static boolean cullBackFaces;
    private static boolean skipDuplicateQuads;
    private static TransparentFaces.@Nullable TextureAlpha textureAlpha;
    /** The render type whose texture alpha {@link #textureAlpha()} looks up on first use, or null once looked up */
    private static @Nullable RenderType alphaRenderType;
    private static RenderType lastRenderType;
    private static boolean lastIdempotent;
    private static boolean blockEntitySubmit;
    private static Boolean polygonOffsetTextCulls;

    private ModelCulling() {
    }

    public static void setWorldPass(boolean active) {
        worldPass = active;
        cullBackFaces = false;
    }

    /** Whether the world's features are being prepared */
    public static boolean isWorldPass() {
        return worldPass;
    }

    public static void beginModel(RenderType renderType) {
        cullBackFaces = worldPass && renderType.pipeline().isCull() && Initializer.CONFIG.modelBackFaceCulling;
        if (renderType != lastRenderType) {
            lastRenderType = renderType;
            lastIdempotent = isIdempotent(renderType);
        }
        skipDuplicateQuads = lastIdempotent;
        // Looked up when a cube is written: most submits are instanced and write none
        textureAlpha = null;
        alphaRenderType = worldPass && Initializer.CONFIG.transparentFaceCulling ? renderType : null;
    }

    /**
     * Whether drawing a quad a second time, right after itself, leaves the image unchanged: no blending and no
     * sorting. The repeated fragments have the same depth and color, so they either fail the depth test or write
     * what is already there.
     */
    private static boolean isIdempotent(RenderType renderType) {
        if (renderType.sortOnUpload()) {
            return false;
        }
        for (ColorTargetState target : renderType.pipeline().getColorTargetStates()) {
            if (target != null && target.blendFunction().isPresent()) {
                return false;
            }
        }
        return true;
    }

    /** Whether exact repeats of a quad of the current model may be left out */
    public static boolean skipDuplicateQuads() {
        return skipDuplicateQuads;
    }

    public static void endModel() {
        cullBackFaces = false;
        skipDuplicateQuads = false;
        textureAlpha = null;
        alphaRenderType = null;
    }

    /**
     * The texture alpha of the current model's render type when faces that draw nothing may be left out
     * ({@link TransparentFaces}), or null
     */
    public static TransparentFaces.@Nullable TextureAlpha textureAlpha() {
        RenderType renderType = alphaRenderType;
        if (renderType != null) {
            alphaRenderType = null;
            // Improved transparency draws blending types with OIT pipelines, which aren't tested
            textureAlpha = !Minecraft.getInstance().gameRenderer.useImprovedTransparency() ? TransparentFaces.of(renderType) : null;
        }
        return textureAlpha;
    }

    public static boolean cullBackFaces() {
        return cullBackFaces;
    }

    public static void setBlockEntitySubmit(boolean active) {
        blockEntitySubmit = active;
    }

    /**
     * Whether sign text (polygon offset text) drawn in the z = 0 plane of {@code pose} faces away from every eye
     * position within {@link #TEXT_EYE_MARGIN} of the origin, so the GPU would cull all of its glyphs.
     * <p>
     * Glyph quads go (x0, y0), (x0, y1), (x1, y1), (x1, y0) with x0 &lt; x1 and y0 &lt; y1, so their winding normal
     * {@code (v1 - v0) x (v2 - v0)} is that of (0, 0), (0, 1), (1, 1), which the pose maps to
     * {@code column1 x column0}. As in {@code ModelPartCubeM}, the GPU keeps a face when that normal points
     * towards the eye.
     */
    public static boolean isSignTextHidden(Matrix4f pose) {
        if (!blockEntitySubmit || !Initializer.CONFIG.modelBackFaceCulling || !polygonOffsetTextCulls()) {
            return false;
        }

        float ax = pose.m10(), ay = pose.m11(), az = pose.m12();
        float bx = pose.m00(), by = pose.m01(), bz = pose.m02();
        float nx = ay * bz - az * by;
        float ny = az * bx - ax * bz;
        float nz = ax * by - ay * bx;

        // Distance of the eye behind the text plane, times |n|
        float behind = nx * pose.m30() + ny * pose.m31() + nz * pose.m32();
        return behind > 0.0f && behind * behind >= TEXT_EYE_MARGIN * TEXT_EYE_MARGIN * (nx * nx + ny * ny + nz * nz);
    }

    private static boolean polygonOffsetTextCulls() {
        Boolean culls = polygonOffsetTextCulls;
        if (culls == null) {
            culls = RenderPipelines.TEXT_POLYGON_OFFSET.isCull() && RenderPipelines.TEXT_GRAYSCALE_POLYGON_OFFSET.isCull()
                    && culls(RenderPipelines.OIT_TEXT_POLYGON_OFFSET) && culls(RenderPipelines.OIT_TEXT_GRAYSCALE_POLYGON_OFFSET);
            polygonOffsetTextCulls = culls;
        }
        return culls;
    }

    private static boolean culls(OitPipelineSet set) {
        RenderPipeline[] pipelines = {set.depthBoundsPipeline(), set.transmittancePipeline(), set.accumulatePipeline()};
        for (RenderPipeline pipeline : pipelines) {
            if (!pipeline.isCull()) {
                return false;
            }
        }
        return true;
    }
}
