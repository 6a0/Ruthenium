package net.ruthenium.mixin.render.text;

import net.minecraft.client.gui.Font;
import net.minecraft.client.renderer.feature.TextFeatureRenderer;
import net.minecraft.util.FormattedCharSequence;
import net.ruthenium.render.text.RetainedLabels;
import net.ruthenium.render.text.SignTextCache;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Redirect;

/**
 * Reuses the glyph layout of sign lines across frames, see {@link SignTextCache}. Other text is laid out as in
 * vanilla. Kept texts may be drawn as retained labels ({@link RetainedLabels}).
 */
@Mixin(TextFeatureRenderer.class)
public class TextFeatureRendererM {

    /** The prepared text of a text submit without outline: a kept one may be drawn as a retained label */
    @Redirect(method = "renderText", at = @At(value = "INVOKE", ordinal = 0,
            target = "Lnet/minecraft/client/gui/Font$PreparedText;visit(Lnet/minecraft/client/gui/Font$GlyphVisitor;)V"))
    private static void retainedLabel(Font.PreparedText text, Font.GlyphVisitor visitor) {
        if (!(text instanceof SignTextCache.KeptText kept) || !(visitor instanceof TextFeatureRenderer.GlyphRenderer renderer)
                || !RetainedLabels.submit(kept, renderer)) {
            text.visit(visitor);
        }
    }

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
