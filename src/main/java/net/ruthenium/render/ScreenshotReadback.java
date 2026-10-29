package net.ruthenium.render;

import com.mojang.blaze3d.pipeline.RenderTarget;
import com.mojang.blaze3d.platform.NativeImage;
import com.mojang.blaze3d.systems.RenderSystem;
import com.mojang.renderpearl.api.buffers.GpuBuffer;
import com.mojang.renderpearl.api.buffers.GpuBufferSlice;
import com.mojang.renderpearl.api.textures.GpuTexture;
import net.ruthenium.Initializer;
import org.lwjgl.system.MemoryUtil;

import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.util.function.Consumer;

/**
 * Vanilla's {@code Screenshot.takeScreenshot} at full size (F2, the new world's icon on its first frame, the benchmark's
 * screenshots) copies the read back frame into the image a pixel at a time ({@code ByteBuffer.getInt}, then
 * {@code setPixelABGR} with the alpha set, rows flipped) in a destroy queue callback of the render thread: ~12 ms for a
 * 1920x1080 frame the first time (the icon), ~2.5-3 ms later. This does the same copy a row at a time, two pixels per
 * long: the same bytes (each pixel's int as read in native order, alpha byte set, row {@code height - 1 - y}).
 */
public final class ScreenshotReadback {
    /** The alpha byte of both pixels of a little-endian long */
    private static final long ALPHA_PAIR = 0xFF000000FF000000L;

    private ScreenshotReadback() {
    }

    /**
     * Vanilla's {@code takeScreenshot(target, downscaleFactor, callback)} with the bulk copy: false (vanilla takes it) when
     * the switch is off, the shot is downscaled or the color format isn't 4 bytes per texel
     */
    public static boolean take(RenderTarget target, int downscaleFactor, Consumer<NativeImage> callback) {
        GpuTexture sourceTexture = target.getColorTexture();
        if (!Initializer.CONFIG.fastScreenshots || downscaleFactor != 1 || sourceTexture == null
                || sourceTexture.getFormat().blockSize() != 4 || ByteOrder.nativeOrder() != ByteOrder.LITTLE_ENDIAN) {
            return false;
        }
        int width = target.width;
        int height = target.height;
        GpuBuffer buffer = RenderSystem.getDevice().createBuffer(() -> "Screenshot buffer", 9, (long) width * height * 4);
        RenderSystem.getDevice().createCommandEncoder().copyTextureToBuffer(sourceTexture, buffer, 0L, () -> {
            try (GpuBufferSlice.MappedView read = buffer.map(true, false)) {
                NativeImage image = new NativeImage(width, height, false);
                copy(read.data(), image, width, height);
                callback.accept(image);
            }
            buffer.close();
        }, 0);
        return true;
    }

    /** The read back pixels into the image, rows flipped and alpha set, as vanilla's loop writes them */
    static void copy(ByteBuffer data, NativeImage image, int width, int height) {
        if (data.order() != ByteOrder.nativeOrder() || image.format() != NativeImage.Format.RGBA) {
            // Vanilla's loop (ByteBuffer.getInt reads in the buffer's order)
            for (int y = 0; y < height; y++) {
                for (int x = 0; x < width; x++) {
                    image.setPixelABGR(x, height - y - 1, data.getInt((x + y * width) * 4) | 0xFF000000);
                }
            }
            return;
        }
        if ((long) width * height * 4 > data.capacity()) {
            throw new IndexOutOfBoundsException("Screenshot buffer smaller than " + width + "x" + height);
        }
        long source = MemoryUtil.memAddress0(data);
        long target = image.getPointer();
        long rowBytes = (long) width * 4;
        int pairs = width >> 1;
        for (int y = 0; y < height; y++) {
            long from = source + y * rowBytes;
            long to = target + (height - 1 - y) * rowBytes;
            for (int i = 0; i < pairs; i++) {
                MemoryUtil.memPutLong(to + i * 8L, MemoryUtil.memGetLong(from + i * 8L) | ALPHA_PAIR);
            }
            if ((width & 1) != 0) {
                long last = rowBytes - 4;
                MemoryUtil.memPutInt(to + last, MemoryUtil.memGetInt(from + last) | 0xFF000000);
            }
        }
    }
}
