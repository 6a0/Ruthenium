package net.ruthenium.gametest.mixin;

import com.llamalad7.mixinextras.sugar.Local;
import net.minecraft.client.renderer.GameRenderer;
import net.ruthenium.gametest.OcclusionProbe;
import org.joml.Matrix4f;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/** {@link OcclusionProbe}: the level's final projection, view bobbing and nausea included */
@Mixin(GameRenderer.class)
public class OcclusionProbeProjectionMixin {
    @Inject(method = "renderLevel", at = @At(value = "INVOKE",
            target = "Lcom/mojang/blaze3d/systems/RenderSystem;setProjectionMatrix(Lcom/mojang/renderpearl/api/buffers/GpuBufferSlice;Lcom/mojang/blaze3d/ProjectionType;)V",
            ordinal = 0))
    private void captureProjection(CallbackInfo ci, @Local(ordinal = 0) Matrix4f projectionMatrix) {
        if (OcclusionProbe.ENABLED) {
            OcclusionProbe.setProjection(projectionMatrix);
        }
    }
}
