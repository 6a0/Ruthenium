package net.vulkanmod.mixin.render.vertex;

import net.minecraft.client.renderer.StagedVertexBuffer;
import net.vulkanmod.render.vertex.ParallelUploadCopy;
import org.lwjgl.system.MemoryUtil;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.Redirect;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import java.nio.BufferOverflowException;
import java.nio.ByteBuffer;

/**
 * Copies vertex data into the mapped staging buffer on several threads ({@link ParallelUploadCopy}), overlapping
 * with the sorting of translucent draws.
 */
@Mixin(StagedVertexBuffer.class)
public class StagedVertexBufferM {
    private static final String UPLOAD = "uploadDrawsToBuffers";

    /** Records the copy instead of doing it */
    @Redirect(method = UPLOAD, at = @At(value = "INVOKE", target = "Ljava/nio/ByteBuffer;put(Ljava/nio/ByteBuffer;)Ljava/nio/ByteBuffer;"))
    private ByteBuffer deferCopy(ByteBuffer target, ByteBuffer source) {
        int size = source.remaining();
        if (size > target.remaining()) {
            throw new BufferOverflowException();
        }

        ParallelUploadCopy.add(MemoryUtil.memAddress(source), MemoryUtil.memAddress(target), size);
        target.position(target.position() + size);
        source.position(source.limit());
        return target;
    }

    /** Sorting only reads vertex data, so the copy can run meanwhile */
    @Inject(method = UPLOAD, at = @At(value = "INVOKE", target = "Lnet/minecraft/client/renderer/StagedVertexBuffer;decodeSortingPoints(Lnet/minecraft/client/renderer/StagedVertexBuffer$Draw;)Lcom/mojang/blaze3d/vertex/CompactVectorArray;"))
    private void startCopy(CallbackInfo ci) {
        ParallelUploadCopy.start();
    }

    /** Before the staging buffer is unmapped (both the normal and the exceptional exit of the try block) */
    @Inject(method = UPLOAD, at = @At(value = "INVOKE", target = "Lcom/mojang/renderpearl/api/buffers/GpuBufferSlice$MappedView;close()V"))
    private void finishCopy(CallbackInfo ci) {
        ParallelUploadCopy.finish();
    }
}
