package dev.ruthenium.bench.mixin;

import net.minecraft.client.DeltaTracker;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.GuiGraphicsExtractor;
import net.minecraft.client.gui.Hud;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/**
 * For a picture of the world with the player's hand and the outline of the block it looks at, and
 * nothing else of the HUD (crosshair, hotbar, vignette): the game draws the hand and the outline
 * only while its HUD is shown, so the HUD stays on and draws nothing.
 */
@Mixin(Hud.class)
public abstract class HudMixin {
    @Inject(method = "extractRenderState", at = @At("HEAD"), cancellable = true)
    private void ruthenium$handOnly(GuiGraphicsExtractor graphics, DeltaTracker deltaTracker, CallbackInfo ci) {
        if (Boolean.getBoolean("ruthenium.bench.hand")) {
            Minecraft.getInstance().gameRenderer.gameRenderState().guiRenderState.isHudHidden = false;
            ci.cancel();
        }
    }
}
