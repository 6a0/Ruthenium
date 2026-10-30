package net.ruthenium.gametest.mixin;

import com.mojang.renderpearl.api.commands.RenderPass;
import net.minecraft.client.renderer.LevelRenderer;
import net.minecraft.client.renderer.chunk.ChunkSectionsToRender;
import net.minecraft.client.renderer.feature.FeatureRenderDispatcher;
import net.ruthenium.gametest.GpuTimelineProbe;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/** {@link GpuTimelineProbe}: marks inside the main pass (each names the part that starts there) */
@Mixin(LevelRenderer.class)
public class GpuTimelineLevelMixin {
    private static final String RENDER_GROUP = "Lnet/minecraft/client/renderer/chunk/ChunkSectionsToRender;renderGroup(Lnet/minecraft/client/renderer/chunk/ChunkSectionLayerGroup;Lcom/mojang/renderpearl/api/commands/RenderPass;Lcom/mojang/renderpearl/api/textures/GpuSampler;Lcom/mojang/renderpearl/api/textures/GpuTextureView;Z)V";

    @Inject(method = "executeSolid", at = @At("HEAD"))
    private void tlSolid(ChunkSectionsToRender sections, FeatureRenderDispatcher.PreparedFrame frame, RenderPass pass, CallbackInfo ci) {
        GpuTimelineProbe.markPass(pass, "opaqueTerrain");
    }

    @Inject(method = "executeSolid", at = @At(value = "INVOKE", shift = At.Shift.AFTER, target = RENDER_GROUP))
    private void tlSolidFeatures(ChunkSectionsToRender sections, FeatureRenderDispatcher.PreparedFrame frame, RenderPass pass, CallbackInfo ci) {
        GpuTimelineProbe.markPass(pass, "solidFeatures");
    }

    @Inject(method = "executeClassicTransparency", at = @At("HEAD"))
    private void tlTranslucentFeatures(ChunkSectionsToRender sections, FeatureRenderDispatcher.PreparedFrame frame, RenderPass pass, CallbackInfo ci) {
        GpuTimelineProbe.markPass(pass, "translucentFeatures");
    }

    @Inject(method = "executeClassicTransparency", at = @At(value = "INVOKE", target = RENDER_GROUP))
    private void tlTranslucentTerrain(ChunkSectionsToRender sections, FeatureRenderDispatcher.PreparedFrame frame, RenderPass pass, CallbackInfo ci) {
        GpuTimelineProbe.markPass(pass, "translucentTerrain");
    }

    @Inject(method = "executeClassicTransparency", at = @At(value = "INVOKE", shift = At.Shift.AFTER, target = RENDER_GROUP))
    private void tlAfterTerrain(ChunkSectionsToRender sections, FeatureRenderDispatcher.PreparedFrame frame, RenderPass pass, CallbackInfo ci) {
        GpuTimelineProbe.markPass(pass, "translucentAfterTerrain");
    }

    @Inject(method = "executeClassicTransparency", at = @At(value = "INVOKE", target = "Lnet/minecraft/client/renderer/CloudRenderer;render(Lnet/minecraft/client/CloudStatus;Lcom/mojang/renderpearl/api/commands/RenderPass;)V"))
    private void tlClouds(ChunkSectionsToRender sections, FeatureRenderDispatcher.PreparedFrame frame, RenderPass pass, CallbackInfo ci) {
        GpuTimelineProbe.markPass(pass, "clouds");
    }

    @Inject(method = "executeClassicTransparency", at = @At(value = "INVOKE", target = "Lnet/minecraft/client/renderer/WeatherEffectRenderer;render(Lnet/minecraft/client/renderer/state/level/WeatherRenderState;Lcom/mojang/renderpearl/api/commands/RenderPass;)V"))
    private void tlWeather(ChunkSectionsToRender sections, FeatureRenderDispatcher.PreparedFrame frame, RenderPass pass, CallbackInfo ci) {
        GpuTimelineProbe.markPass(pass, "weatherBorder");
    }

    @Inject(method = "executeClassicTransparency", at = @At("RETURN"))
    private void tlEnd(ChunkSectionsToRender sections, FeatureRenderDispatcher.PreparedFrame frame, RenderPass pass, CallbackInfo ci) {
        GpuTimelineProbe.markPass(pass, "mainEnd");
    }
}
