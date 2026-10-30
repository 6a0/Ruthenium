package net.ruthenium.mixin.render.text;

import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.llamalad7.mixinextras.injector.wrapoperation.WrapOperation;
import com.mojang.blaze3d.font.GlyphProvider;
import it.unimi.dsi.fastutil.ints.Int2ObjectMap;
import net.minecraft.client.gui.font.FontOption;
import net.minecraft.client.gui.font.FontSet;
import net.ruthenium.Initializer;
import net.ruthenium.interfaces.LazyGlyphWidthsHolder;
import net.ruthenium.render.blockentity.RetainedBlockEntities;
import net.ruthenium.render.text.LazyGlyphWidths;
import net.ruthenium.render.text.SignTextCache;
import org.jspecify.annotations.Nullable;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

import java.util.List;
import java.util.Set;

/**
 * Prepared text and retained sign text hold baked glyphs, which a font set replaces when it reloads.
 * <p>
 * Switch {@code lazyGlyphWidths} ({@link LazyGlyphWidths}): {@code selectProviders} no longer fills
 * {@code glyphsByWidth} (it still finds the providers in use), and {@code getRandomGlyph}, its only reader, reads the
 * lazily built table instead. The switch is read when a font set reloads.
 */
@Mixin(FontSet.class)
public class FontSetM implements LazyGlyphWidthsHolder {
    @Unique
    private volatile @Nullable LazyGlyphWidths ruthenium$lazyWidths;

    @Inject(method = "resetTextures", at = @At("HEAD"))
    private void invalidateSignText(CallbackInfo ci) {
        SignTextCache.invalidate();
        RetainedBlockEntities.clear();
    }

    @Inject(method = "selectProviders", at = @At("HEAD"))
    private void ruthenium$deferGlyphWidths(List<GlyphProvider.Conditional> providers, Set<FontOption> options,
                                            CallbackInfoReturnable<List<GlyphProvider>> cir) {
        this.ruthenium$lazyWidths = Initializer.CONFIG.lazyGlyphWidths ? new LazyGlyphWidths(providers, options) : null;
    }

    /**
     * In the lambda of {@code supportedGlyphs.forEach}: once the first provider with a glyph for the code point has
     * been added to the used providers (vanilla then adds the code point to its width and breaks), return.
     */
    @Inject(method = "lambda$selectProviders$0", at = @At(value = "INVOKE",
            target = "Lcom/mojang/blaze3d/font/UnbakedGlyph;info()Lcom/mojang/blaze3d/font/GlyphInfo;", ordinal = 0),
            cancellable = true)
    private void ruthenium$skipGlyphWidth(List<?> selectedProviders, Set<?> usedProviders, int codepoint, CallbackInfo ci) {
        if (this.ruthenium$lazyWidths != null) {
            ci.cancel();
        }
    }

    @WrapOperation(method = "getRandomGlyph", at = @At(value = "INVOKE",
            target = "Lit/unimi/dsi/fastutil/ints/Int2ObjectMap;get(I)Ljava/lang/Object;"))
    private Object ruthenium$lazyGlyphWidths(Int2ObjectMap<?> glyphsByWidth, int width, Operation<Object> original) {
        LazyGlyphWidths lazy = this.ruthenium$lazyWidths;
        return lazy != null ? lazy.widths().get(width) : original.call(glyphsByWidth, width);
    }

    @Override
    public @Nullable LazyGlyphWidths ruthenium$lazyGlyphWidths() {
        return this.ruthenium$lazyWidths;
    }
}
