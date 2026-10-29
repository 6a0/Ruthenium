package net.ruthenium.mixin.render.sky;

import com.mojang.renderpearl.api.commands.RenderPass;
import com.mojang.renderpearl.api.pipeline.RenderPipeline;
import net.minecraft.client.renderer.CloudRenderer;
import net.minecraft.client.renderer.MappableRingBuffer;
import net.ruthenium.render.sky.CloudFaceTexture;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.ModifyArg;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import java.nio.ByteBuffer;

/** Draws the clouds from {@link CloudFaceTexture} instead of vanilla's texel buffer when its switch is on */
@Mixin(CloudRenderer.class)
public class CloudRendererM {
    private static final String PREPARE = "prepare(ILnet/minecraft/client/CloudStatus;FILnet/minecraft/world/phys/Vec3;JF)V";
    private static final String BUILD_MESH =
            "Lnet/minecraft/client/renderer/CloudRenderer;buildMesh(Lnet/minecraft/client/renderer/CloudRenderer$RelativeCameraPos;Ljava/nio/ByteBuffer;IIZI)V";

    @Shadow private boolean needsRebuild;
    @Shadow private int quadCount;
    @Shadow @Final private MappableRingBuffer ubo;

    @Unique private final CloudFaceTexture ruthenium$faces = new CloudFaceTexture();

    @Inject(method = PREPARE, at = @At("HEAD"))
    private void ruthenium$buildForTexture(CallbackInfo ci) {
        if (this.ruthenium$faces.needsBuild()) {
            this.needsRebuild = true;
        }
    }

    @ModifyArg(method = PREPARE, at = @At(value = "INVOKE", target = BUILD_MESH), index = 1)
    private ByteBuffer ruthenium$buildInto(ByteBuffer faceBuffer) {
        return this.ruthenium$faces.beforeBuild(faceBuffer);
    }

    @Inject(method = PREPARE, at = @At(value = "INVOKE", target = BUILD_MESH, shift = At.Shift.AFTER))
    private void ruthenium$uploadFaces(CallbackInfo ci) {
        this.ruthenium$faces.afterBuild();
    }

    @Inject(method = "render(Lcom/mojang/renderpearl/api/commands/RenderPass;Lcom/mojang/renderpearl/api/pipeline/RenderPipeline;)V",
            at = @At("HEAD"), cancellable = true)
    private void ruthenium$drawFromTexture(RenderPass renderPass, RenderPipeline renderPipeline, CallbackInfo ci) {
        if (this.ruthenium$faces.draw(renderPass, renderPipeline, this.ubo, this.quadCount)) {
            ci.cancel();
        }
    }

    @Inject(method = "close", at = @At("RETURN"))
    private void ruthenium$close(CallbackInfo ci) {
        this.ruthenium$faces.close();
    }
}
