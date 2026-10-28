package net.thallium.mixin.render.text;

import net.minecraft.client.gui.Font;
import net.minecraft.client.renderer.feature.TextFeatureRenderer;
import net.minecraft.util.FormattedCharSequence;
import net.thallium.render.text.SignTextCache;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Redirect;

/**
 * Reuses the glyph layout of sign lines across frames, see {@link SignTextCache}. Other text is laid out as in
 * vanilla.
 */
@Mixin(TextFeatureRenderer.class)
public class TextFeatureRendererM {

    @Redirect(method = "renderText", require = 2, at = @At(value = "INVOKE",
            target = "Lnet/minecraft/client/gui/Font;prepareText(Lnet/minecraft/util/FormattedCharSequence;FFIZZI)Lnet/minecraft/client/gui/Font$PreparedText;"))
    private static Font.PreparedText cachedText(Font font, FormattedCharSequence line, float x, float y, int color,
                                                boolean drawShadow, boolean includeEmpty, int backgroundColor) {
        return SignTextCache.prepareText(font, line, x, y, color, drawShadow, includeEmpty, backgroundColor);
    }

    @Redirect(method = "renderText", at = @At(value = "INVOKE",
            target = "Lnet/minecraft/client/gui/Font;prepare8xTextOutline(Lnet/minecraft/util/FormattedCharSequence;FFI)Lnet/minecraft/client/gui/Font$PreparedText;"))
    private static Font.PreparedText cachedOutline(Font font, FormattedCharSequence line, float x, float y, int outlineColor) {
        return SignTextCache.prepare8xTextOutline(font, line, x, y, outlineColor);
    }
}
