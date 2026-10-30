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
 */
public final class FrameClear {
    private static @Nullable GpuTexture pendingColor;
    private static @Nullable GpuTexture pendingDepth;
    private static final Vector4f PENDING_CLEAR_COLOR = new Vector4f();
    private static double pendingClearDepth;

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

    /** Before a full clear of a color and a depth texture: drops the deferred clear when it is of the same two, else performs it if it touches either */
    public static void beforeFullClear(GpuTexture color, GpuTexture depth) {
        if (pendingColor == null) {
            return;
        }
        if (color == pendingColor && depth == pendingDepth) {
            pendingColor = null;
            pendingDepth = null;
        } else {
            beforeUse(color);
            beforeUse(depth);
        }
    }

    /** Before any other use of a texture: performs the deferred clear first if it is of that texture */
    public static void beforeUse(@Nullable GpuTexture texture) {
        if (pendingColor != null && texture != null && (texture == pendingColor || texture == pendingDepth)) {
            flush();
        }
    }

    /** Performs the deferred clear, if any */
    public static void flush() {
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
}
