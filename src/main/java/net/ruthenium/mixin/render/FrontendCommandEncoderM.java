package net.ruthenium.mixin.render;

import com.mojang.blaze3d.platform.NativeImage;
import com.mojang.renderpearl.api.buffers.GpuBuffer;
import com.mojang.renderpearl.api.buffers.GpuBufferSlice;
import com.mojang.renderpearl.api.commands.RenderPass;
import com.mojang.renderpearl.api.commands.RenderPassDescriptor;
import com.mojang.renderpearl.api.textures.GpuTexture;
import com.mojang.renderpearl.frontend.FrontendCommandEncoder;
import net.ruthenium.render.FrameClear;
import org.joml.Vector4fc;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

import java.nio.ByteBuffer;
import java.util.Optional;
import java.util.OptionalDouble;

/**
 * {@link FrameClear}: performs a deferred clear of the main target before any other use of its textures, and drops it at a full
 * clear of both
 */
@Mixin(FrontendCommandEncoder.class)
public class FrontendCommandEncoderM {
    @Inject(method = "createRenderPass(Lcom/mojang/renderpearl/api/commands/RenderPassDescriptor;)Lcom/mojang/renderpearl/api/commands/RenderPass;",
            at = @At("HEAD"))
    private void ruthenium$beforePass(RenderPassDescriptor descriptor, CallbackInfoReturnable<RenderPass> cir) {
        for (RenderPassDescriptor.Attachment<Optional<Vector4fc>> attachment : descriptor.colorAttachments()) {
            if (attachment != null) {
                FrameClear.beforeUse(attachment.textureView().texture());
            }
        }
        RenderPassDescriptor.Attachment<OptionalDouble> depth = descriptor.depthAttachment();
        if (depth != null) {
            FrameClear.beforeUse(depth.textureView().texture());
        }
    }

    @Inject(method = "clearColorAndDepthTextures(Lcom/mojang/renderpearl/api/textures/GpuTexture;Lorg/joml/Vector4fc;Lcom/mojang/renderpearl/api/textures/GpuTexture;D)V",
            at = @At("HEAD"))
    private void ruthenium$beforeFullClear(GpuTexture color, Vector4fc clearColor, GpuTexture depth, double clearDepth, CallbackInfo ci) {
        FrameClear.beforeFullClear(color, depth);
    }

    @Inject(method = "clearColorAndDepthTextures(Lcom/mojang/renderpearl/api/textures/GpuTexture;Lorg/joml/Vector4fc;Lcom/mojang/renderpearl/api/textures/GpuTexture;DIIIII)V",
            at = @At("HEAD"))
    private void ruthenium$beforeRegionClear(GpuTexture color, Vector4fc clearColor, GpuTexture depth, double clearDepth, int x, int y, int width,
                                             int height, int mipLevel, CallbackInfo ci) {
        FrameClear.beforeUse(color);
        FrameClear.beforeUse(depth);
    }

    @Inject(method = "clearColorTexture", at = @At("HEAD"))
    private void ruthenium$beforeColorClear(GpuTexture color, Vector4fc clearColor, CallbackInfo ci) {
        FrameClear.beforeUse(color);
    }

    @Inject(method = "clearDepthTexture", at = @At("HEAD"))
    private void ruthenium$beforeDepthClear(GpuTexture depth, double clearDepth, CallbackInfo ci) {
        FrameClear.beforeUse(depth);
    }

    @Inject(method = "writeToTexture(Lcom/mojang/renderpearl/api/textures/GpuTexture;Lcom/mojang/blaze3d/platform/NativeImage;IIII)V", at = @At("HEAD"))
    private void ruthenium$beforeImageWrite(GpuTexture destination, NativeImage source, int mipLevel, int layer, int x, int y, CallbackInfo ci) {
        FrameClear.beforeUse(destination);
    }

    @Inject(method = "writeToTexture(Lcom/mojang/renderpearl/api/textures/GpuTexture;Ljava/nio/ByteBuffer;IIIIII)V", at = @At("HEAD"))
    private void ruthenium$beforeBufferWrite(GpuTexture destination, ByteBuffer source, int mipLevel, int layer, int x, int y, int width, int height,
                                             CallbackInfo ci) {
        FrameClear.beforeUse(destination);
    }

    @Inject(method = "copyBufferToTexture", at = @At("HEAD"))
    private void ruthenium$beforeBufferCopy(GpuBufferSlice source, int sourceX, int sourceY, int sourceWidth, int sourceHeight, GpuTexture destination,
                                            int destinationX, int destinationY, int copyWidth, int copyHeight, int mipLevel, int arrayLayer,
                                            CallbackInfo ci) {
        FrameClear.beforeUse(destination);
    }

    @Inject(method = "copyTextureToBuffer(Lcom/mojang/renderpearl/api/textures/GpuTexture;Lcom/mojang/renderpearl/api/buffers/GpuBuffer;JLjava/lang/Runnable;I)V",
            at = @At("HEAD"))
    private void ruthenium$beforeReadback(GpuTexture source, GpuBuffer destination, long offset, Runnable callback, int mipLevel, CallbackInfo ci) {
        FrameClear.beforeUse(source);
    }

    @Inject(method = "copyTextureToBuffer(Lcom/mojang/renderpearl/api/textures/GpuTexture;Lcom/mojang/renderpearl/api/buffers/GpuBuffer;JLjava/lang/Runnable;IIIII)V",
            at = @At("HEAD"))
    private void ruthenium$beforeRegionReadback(GpuTexture source, GpuBuffer destination, long offset, Runnable callback, int mipLevel, int x, int y,
                                                int width, int height, CallbackInfo ci) {
        FrameClear.beforeUse(source);
    }

    @Inject(method = "copyTextureToTexture", at = @At("HEAD"))
    private void ruthenium$beforeTextureCopy(GpuTexture source, GpuTexture destination, int mipLevel, int destX, int destY, int sourceX, int sourceY,
                                             int width, int height, CallbackInfo ci) {
        FrameClear.beforeUse(source);
        FrameClear.beforeUse(destination);
    }
}
