package net.ruthenium.mixin.render;

import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.llamalad7.mixinextras.injector.wrapoperation.WrapOperation;
import com.mojang.renderpearl.api.pipeline.RenderPipeline;
import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.gui.Hud;
import net.minecraft.client.renderer.RenderPipelines;
import net.minecraft.resources.Identifier;
import net.ruthenium.Initializer;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;

/**
 * Skips the vignette when its color is black. Vanilla's vignette pipeline blends the screen with ZERO, ONE_MINUS_SRC_COLOR
 * (color) and ZERO, ONE (alpha): with a black vertex color the fragment color is 0 and every pixel keeps its value, so the
 * full screen quad changes nothing. Its color is black whenever the camera's eye has full light for some 30 s (the
 * vignette brightness decays by 1% per tick to below one 8 bit level) and no world border warning shows: in daylight.
 */
@Mixin(Hud.class)
public class HudVignetteM {
    @WrapOperation(method = "extractVignette", at = @At(value = "INVOKE",
            target = "Lnet/minecraft/client/gui/GuiGraphicsExtractor;blit(Lcom/mojang/renderpearl/api/pipeline/RenderPipeline;Lnet/minecraft/resources/Identifier;IIFFIIIII)V"))
    private void ruthenium$skipBlackVignette(GuiGraphicsExtractor graphics, RenderPipeline pipeline, Identifier texture, int x, int y,
                                             float u, float v, int width, int height, int textureWidth, int textureHeight, int color,
                                             Operation<Void> original) {
        if (Initializer.CONFIG.skipBlackVignette && pipeline == RenderPipelines.VIGNETTE && (color & 0xFFFFFF) == 0) {
            return;
        }
        original.call(graphics, pipeline, texture, x, y, u, v, width, height, textureWidth, textureHeight, color);
    }
}
