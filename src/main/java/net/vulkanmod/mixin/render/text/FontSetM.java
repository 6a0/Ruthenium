package net.vulkanmod.mixin.render.text;

import net.minecraft.client.gui.font.FontSet;
import net.vulkanmod.render.text.SignTextCache;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/**
 * Prepared text holds baked glyphs, which a font set replaces when it reloads.
 */
@Mixin(FontSet.class)
public class FontSetM {

    @Inject(method = "resetTextures", at = @At("HEAD"))
    private void invalidateSignText(CallbackInfo ci) {
        SignTextCache.invalidate();
    }
}
