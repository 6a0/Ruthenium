package net.rubidium.render.text;

import net.minecraft.client.gui.Font;
import net.minecraft.util.FormattedCharSequence;
import net.minecraft.util.FormattedCharSink;

/**
 * Caches the width and prepared glyphs of sign text lines across frames.
 * <p>
 * Vanilla measures every sign line in {@code AbstractSignRenderer.submitSignText} and lays out its glyphs again
 * in {@code TextFeatureRenderer.renderText}, every frame. {@code SignText} keeps its render lines until the text
 * changes, and each line is wrapped in a {@link Line} when it is created there, so the cache travels with the
 * line: no lookups, and nothing to evict. Other text (name tags etc.) is never wrapped.
 * <p>
 * A {@link Font.PreparedText} only depends on the sequence, its parameters and the font's glyphs, and visiting it
 * does not change it. Lines with obfuscated characters pick random glyphs on every layout, so they are not cached.
 * Everything is invalidated when a font set reloads its glyphs. Only used from the render thread.
 */
public final class SignTextCache {
    private static int generation;

    private SignTextCache() {
    }

    public static FormattedCharSequence wrap(FormattedCharSequence line) {
        return new Line(line);
    }

    public static int width(Font font, FormattedCharSequence line) {
        if (line instanceof Line cached) {
            return cached.width(font);
        }

        return font.width(line);
    }

    public static Font.PreparedText prepareText(Font font, FormattedCharSequence line, float x, float y, int color,
                                                boolean drawShadow, boolean includeEmpty, int backgroundColor) {
        if (!(line instanceof Line cached) || !cached.isCacheable(font)) {
            return font.prepareText(line, x, y, color, drawShadow, includeEmpty, backgroundColor);
        }

        Prepared prepared = drawShadow ? cached.shadowed : cached.plain;
        if (prepared != null && prepared.matches(x, y, color, includeEmpty, backgroundColor)) {
            return prepared.text;
        }

        prepared = new Prepared(x, y, color, includeEmpty, backgroundColor,
                font.prepareText(line, x, y, color, drawShadow, includeEmpty, backgroundColor));
        if (drawShadow) {
            cached.shadowed = prepared;
        } else {
            cached.plain = prepared;
        }
        return prepared.text;
    }

    public static Font.PreparedText prepare8xTextOutline(Font font, FormattedCharSequence line, float x, float y, int outlineColor) {
        if (!(line instanceof Line cached) || !cached.isCacheable(font)) {
            return font.prepare8xTextOutline(line, x, y, outlineColor);
        }

        Prepared prepared = cached.outline;
        if (prepared != null && prepared.matches(x, y, outlineColor, false, 0)) {
            return prepared.text;
        }

        prepared = new Prepared(x, y, outlineColor, false, 0, font.prepare8xTextOutline(line, x, y, outlineColor));
        cached.outline = prepared;
        return prepared.text;
    }

    public static void invalidate() {
        generation++;
    }

    private static final class Line implements FormattedCharSequence {
        private final FormattedCharSequence delegate;
        private Boolean obfuscated;

        private Font font;
        private int generation = -1;
        private int width;
        private Prepared plain;
        private Prepared shadowed;
        private Prepared outline;

        Line(FormattedCharSequence delegate) {
            this.delegate = delegate;
        }

        @Override
        public boolean accept(FormattedCharSink sink) {
            return this.delegate.accept(sink);
        }

        int width(Font font) {
            this.validate(font);
            if (this.width < 0) {
                this.width = font.width(this.delegate);
            }
            return this.width;
        }

        boolean isCacheable(Font font) {
            this.validate(font);
            if (this.obfuscated == null) {
                boolean[] found = new boolean[1];
                this.delegate.accept((position, style, codepoint) -> {
                    if (style.isObfuscated()) {
                        found[0] = true;
                        return false;
                    }
                    return true;
                });
                this.obfuscated = found[0];
            }
            return !this.obfuscated;
        }

        private void validate(Font font) {
            if (this.font != font || this.generation != SignTextCache.generation) {
                this.font = font;
                this.generation = SignTextCache.generation;
                this.width = -1;
                this.plain = null;
                this.shadowed = null;
                this.outline = null;
            }
        }
    }

    private record Prepared(float x, float y, int color, boolean includeEmpty, int backgroundColor, Font.PreparedText text) {
        boolean matches(float x, float y, int color, boolean includeEmpty, int backgroundColor) {
            return Float.compare(this.x, x) == 0 && Float.compare(this.y, y) == 0 && this.color == color
                    && this.includeEmpty == includeEmpty && this.backgroundColor == backgroundColor;
        }
    }
}
