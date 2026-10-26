package net.vulkanmod.mixin.render.text;

import net.minecraft.client.gui.Font;
import net.minecraft.client.renderer.blockentity.AbstractSignRenderer;
import net.minecraft.util.FormattedCharSequence;
import net.vulkanmod.render.text.SignTextCache;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Redirect;

/**
 * Measures sign lines through {@link SignTextCache}, which also registers them for the cached layout in
 * {@link TextFeatureRendererM}.
 */
@Mixin(AbstractSignRenderer.class)
public class AbstractSignRendererM {

    @Redirect(method = "submitSignText", at = @At(value = "INVOKE",
            target = "Lnet/minecraft/client/gui/Font;width(Lnet/minecraft/util/FormattedCharSequence;)I"))
    private int cachedWidth(Font font, FormattedCharSequence line) {
        return SignTextCache.width(font, line);
    }
}
