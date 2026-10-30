package net.ruthenium.gametest.mixin;

import com.mojang.renderpearl.api.commands.RenderPass;
import net.minecraft.client.renderer.LevelRenderer;
import net.minecraft.client.renderer.chunk.ChunkSectionsToRender;
import net.minecraft.client.renderer.feature.FeatureRenderDispatcher;
import net.ruthenium.gametest.OcclusionProbe;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/** {@link OcclusionProbe}: the captured frame leaves out its translucent pass, and its depth is read after the opaque one */
@Mixin(LevelRenderer.class)
public class OcclusionProbeLevelMixin {
    @Inject(method = "executeClassicTransparency", at = @At("HEAD"), cancellable = true)
    private void skipTranslucent(ChunkSectionsToRender chunkSectionsToRender, FeatureRenderDispatcher.PreparedFrame featureFrame,
                                 RenderPass renderPass, CallbackInfo ci) {
        if (OcclusionProbe.ENABLED && OcclusionProbe.isCapturing()) {
            ci.cancel();
        }
    }

    @Inject(method = "executeOutline", at = @At("HEAD"))
    private void readDepth(FeatureRenderDispatcher.PreparedFrame featureFrame, CallbackInfo ci) {
        if (OcclusionProbe.ENABLED) {
            OcclusionProbe.afterOpaquePass();
        }
    }
}
