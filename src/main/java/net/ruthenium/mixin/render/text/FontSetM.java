package net.ruthenium.mixin.render.text;

import net.minecraft.client.gui.font.FontSet;
import net.ruthenium.render.blockentity.RetainedBlockEntities;
import net.ruthenium.render.text.SignTextCache;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/**
 * Prepared text and retained sign text hold baked glyphs, which a font set replaces when it reloads.
 */
@Mixin(FontSet.class)
public class FontSetM {

    @Inject(method = "resetTextures", at = @At("HEAD"))
    private void invalidateSignText(CallbackInfo ci) {
        SignTextCache.invalidate();
        RetainedBlockEntities.clear();
    }
}
