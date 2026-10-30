package net.ruthenium.render;

import com.mojang.blaze3d.systems.RenderSystem;
import com.mojang.renderpearl.api.textures.GpuTexture;
import net.ruthenium.Initializer;
import org.joml.Vector4f;
import org.joml.Vector4fc;
import org.jspecify.annotations.Nullable;

/**
 * Switch {@code skipFrameClear}: {@code GameRenderer.render} clears the main target's color and depth at the start of every
 * frame, and when the level is drawn its frame graph's "clear" pass clears both again before anything else touches them, so the
 * first clear is never seen. It costs a clear on the GPU (~5 us per frame on Vulkan at 1080p) and on OpenGL ~10-15 us of driver
 * time on the render thread.
 * <p>
 * With the switch on, that first clear is only remembered ({@link #defer}). A full clear of the same two textures (the clear
 * pass) drops it, since it overwrites everything the first clear wrote. Any other use of either texture before that (a render
 * pass with it as an attachment, another clear, a copy or upload, the swapchain blit) first performs the remembered clear, so
 * whatever comes first sees the target as vanilla would have left it. Render thread only.
 * <p>
 * The same goes for the depth clear before the hand ({@code GameRenderer.render3dHud}, main target only): when no hand, screen
 * effect or 3D crosshair draws with that depth (spectators, third person, hidden HUD), the GUI's own depth clear comes first and
 * makes it unseen; otherwise the hand's render pass performs it first.
 */
public final class FrameClear {
    private static @Nullable GpuTexture pendingColor;
    private static @Nullable GpuTexture pendingDepth;
    private static final Vector4f PENDING_CLEAR_COLOR = new Vector4f();
    private static double pendingClearDepth;
    /** A deferred depth-only clear ({@link #deferDepth}) */
    private static @Nullable GpuTexture pendingDepthOnly;
    private static double pendingDepthOnlyValue;

    private FrameClear() {
    }

    /**
     * {@code GameRenderer.render}'s clear of the main target, when the level will be drawn
     *
     * @return true if the clear was deferred, false if it has to run now
     */
    public static boolean defer(GpuTexture color, Vector4fc clearColor, GpuTexture depth, double clearDepth) {
        if (!Initializer.CONFIG.skipFrameClear) {
            return false;
        }
        flush();
        pendingColor = color;
        pendingDepth = depth;
        PENDING_CLEAR_COLOR.set(clearColor);
        pendingClearDepth = clearDepth;
        return true;
    }

    /**
     * The depth clear before the hand ({@code GameRenderer.render3dHud}): when nothing draws with that depth before the GUI's own
     * depth clear (no hand, screen effect or 3D crosshair: spectators, third person, hidden HUD), that clear makes it unseen
     *
     * @return true if the clear was deferred, false if it has to run now
     */
    public static boolean deferDepth(GpuTexture depth, double clearDepth) {
        if (!Initializer.CONFIG.skipFrameClear) {
            return false;
        }
        flush();
        pendingDepthOnly = depth;
        pendingDepthOnlyValue = clearDepth;
        return true;
    }

    /** Before a full clear of a color and a depth texture: drops deferred clears it overwrites, performs those it only touches */
    public static void beforeFullClear(GpuTexture color, GpuTexture depth) {
        if (pendingDepthOnly != null && depth == pendingDepthOnly) {
            pendingDepthOnly = null;
        }
        if (pendingColor != null && color == pendingColor && depth == pendingDepth) {
            pendingColor = null;
            pendingDepth = null;
        }
        beforeUse(color);
        beforeUse(depth);
    }

    /** Before a full clear of a depth texture: drops a deferred depth-only clear of it, performs a deferred clear it only touches */
    public static void beforeDepthClear(GpuTexture depth) {
        if (pendingDepthOnly != null && depth == pendingDepthOnly) {
            pendingDepthOnly = null;
        }
        beforeUse(depth);
    }

    /** Before any other use of a texture: performs the deferred clears of that texture first */
    public static void beforeUse(@Nullable GpuTexture texture) {
        if (texture == null) {
            return;
        }
        if (pendingColor != null && (texture == pendingColor || texture == pendingDepth)) {
            flushFrame();
        }
        if (pendingDepthOnly != null && texture == pendingDepthOnly) {
            flushDepth();
        }
    }

    /** Performs the deferred clears, if any */
    public static void flush() {
        flushFrame();
        flushDepth();
    }

    private static void flushFrame() {
        GpuTexture color = pendingColor;
        GpuTexture depth = pendingDepth;
        if (color == null || depth == null) {
            return;
        }
        pendingColor = null;
        pendingDepth = null;
        if (!color.isClosed() && !depth.isClosed()) {
            RenderSystem.getDevice().createCommandEncoder().clearColorAndDepthTextures(color, PENDING_CLEAR_COLOR, depth, pendingClearDepth);
        }
    }

    private static void flushDepth() {
        GpuTexture depth = pendingDepthOnly;
        if (depth == null) {
            return;
        }
        pendingDepthOnly = null;
        if (!depth.isClosed()) {
            RenderSystem.getDevice().createCommandEncoder().clearDepthTexture(depth, pendingDepthOnlyValue);
        }
    }
}
