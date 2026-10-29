package net.ruthenium.mixin.render;

import com.mojang.blaze3d.pipeline.RenderTarget;
import com.mojang.blaze3d.platform.NativeImage;
import net.minecraft.client.Screenshot;
import net.ruthenium.render.ScreenshotReadback;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import java.util.function.Consumer;

/** Full size screenshots copy the read back frame a row at a time ({@link ScreenshotReadback}) */
@Mixin(Screenshot.class)
public class ScreenshotM {
    @Inject(method = "takeScreenshot(Lcom/mojang/blaze3d/pipeline/RenderTarget;ILjava/util/function/Consumer;)V", at = @At("HEAD"), cancellable = true)
    private static void ruthenium$bulkReadback(RenderTarget target, int downscaleFactor, Consumer<NativeImage> callback, CallbackInfo ci) {
        if (ScreenshotReadback.take(target, downscaleFactor, callback)) {
            ci.cancel();
        }
    }
}
