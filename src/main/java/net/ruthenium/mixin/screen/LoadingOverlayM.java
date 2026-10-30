package net.ruthenium.mixin.screen;

import com.llamalad7.mixinextras.injector.ModifyExpressionValue;
import net.minecraft.client.gui.screens.LoadingOverlay;
import net.ruthenium.Initializer;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Constant;
import org.spongepowered.asm.mixin.injection.ModifyConstant;

/**
 * Switch {@code fastLoadingFade}: the loading overlay's timers run {@link #SPEED} times faster. Vanilla keeps the
 * overlay for 2 s after a reload has finished (1 s fading the progress bar, 1 s fading to the screen below), and the
 * game takes no input until it is gone; a reload with the overlay (F3+T, pack changes) also shows it for at least 1 s.
 * At startup that is ~2 s of the time to a usable title screen, and of every reload after the work is done. The
 * animation is the same, only shorter (0.5 s out, 0.25 s minimum), so nothing loads differently.
 */
@Mixin(LoadingOverlay.class)
public class LoadingOverlayM {
    private static final int SPEED = 4;

    @Shadow private long fadeOutStart;
    @Shadow private long fadeInStart;

    @ModifyExpressionValue(method = "extractRenderState", at = @At(value = "INVOKE", target = "Lnet/minecraft/util/Util;getMillis()J"))
    private long ruthenium$fastFade(long now) {
        if (!Initializer.CONFIG.fastLoadingFade) {
            return now;
        }
        // The fade out (2 s) and the fade in (0.5 s) read the time only through this call
        if (this.fadeOutStart > -1L) {
            return this.fadeOutStart + (now - this.fadeOutStart) * SPEED;
        }
        if (this.fadeInStart > -1L) {
            return this.fadeInStart + (now - this.fadeInStart) * SPEED;
        }
        return now;
    }

    @ModifyConstant(method = "isReadyToFadeOut", constant = @Constant(longValue = 1000L))
    private long ruthenium$fastFadeInHold(long millis) {
        return Initializer.CONFIG.fastLoadingFade ? millis / SPEED : millis;
    }
}
