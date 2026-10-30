package net.ruthenium.gametest.loadprobe;

import net.minecraft.client.gui.screens.LoadingOverlay;
import net.ruthenium.gametest.LoadProbe;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/** When the loading overlay sees its reload done and starts fading out (see {@link LoadProbe}) */
@Mixin(LoadingOverlay.class)
public class LoadingOverlayLoadProbeMixin {
    @Shadow private long fadeOutStart;

    @Inject(method = "tick", at = @At("HEAD"))
    private void ruthenium$before(CallbackInfo ci) {
        this.ruthenium$wasFading = this.fadeOutStart != -1L;
    }

    private boolean ruthenium$wasFading;

    @Inject(method = "tick", at = @At("TAIL"))
    private void ruthenium$after(CallbackInfo ci) {
        if (!this.ruthenium$wasFading && this.fadeOutStart != -1L) {
            LoadProbe.mark("loading overlay: reload done, fade out starts");
        }
    }
}
