package net.ruthenium.mixin.screen;

import com.llamalad7.mixinextras.injector.ModifyExpressionValue;
import net.minecraft.client.gui.screens.TitleScreen;
import net.ruthenium.Initializer;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;

/**
 * Switch {@code fastLoadingFade}: the title screen shown after startup fades its buttons in {@link #SPEED} times
 * faster. Vanilla's buttons stay invisible for 1 s after the screen first appears (under the loading overlay's fade)
 * and reach full opacity after 2 s.
 */
@Mixin(TitleScreen.class)
public class TitleScreenFadeM {
    private static final int SPEED = 4;

    @Shadow private long fadeInStart;

    @ModifyExpressionValue(method = "extractRenderState", at = @At(value = "INVOKE", target = "Lnet/minecraft/util/Util;getMillis()J"))
    private long ruthenium$fastFade(long now) {
        if (!Initializer.CONFIG.fastLoadingFade || this.fadeInStart == 0L) {
            return now;
        }
        return this.fadeInStart + (now - this.fadeInStart) * SPEED;
    }
}
