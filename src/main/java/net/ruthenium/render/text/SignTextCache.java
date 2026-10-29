package net.ruthenium.render.text;

import net.minecraft.client.gui.Font;
import net.minecraft.util.FormattedCharSequence;
import net.minecraft.util.FormattedCharSink;
import net.ruthenium.Initializer;

/**
 * Caches the width and prepared glyphs of text lines across frames: sign lines, and since the players and holograms
 * work also name tags and text display lines ({@link EntityTextCache}).
 * <p>
 * Vanilla measures every sign line in {@code AbstractSignRenderer.submitSignText} and lays out its glyphs again
 * in {@code TextFeatureRenderer.renderText}, every frame. {@code SignText} keeps its render lines until the text
 * changes, and each line is wrapped in a {@link Line} when it is created there, so the cache travels with the
 * line: no lookups, and nothing to evict. Text that isn't wrapped is laid out as in vanilla.
 * <p>
 * A {@link Font.PreparedText} only depends on the sequence, its parameters and the font's glyphs, and visiting it
 * does not change it. Lines with obfuscated characters pick random glyphs on every layout, so they are not cached.
 * A line keeps up to {@link #VARIANTS} prepared texts (a name tag is drawn twice, with and without a background).
 * Everything is invalidated when a font set reloads its glyphs. Only used from the render thread.
 */
public final class SignTextCache {
    private static final int VARIANTS = 4;
    /** Where a line comes from: its cache has a switch unless it is a sign's */
    public static final int SIGN = 0, NAME_TAG = 1, TEXT_DISPLAY = 2;
    private static int generation;

    private SignTextCache() {
    }

    public static FormattedCharSequence wrap(FormattedCharSequence line) {
        return new Line(line, SIGN);
    }

    public static FormattedCharSequence wrap(FormattedCharSequence line, int source) {
        return new Line(line, source);
    }

    /** The sequence a line wraps, or null if it isn't one */
    public static FormattedCharSequence unwrap(FormattedCharSequence line) {
        return line instanceof Line cached ? cached.delegate : null;
    }

    public static int width(Font font, FormattedCharSequence line) {
        if (line instanceof Line cached && cached.isEnabled()) {
            return cached.width(font);
        }

        return font.width(line);
    }

    public static int generation() {
        return generation;
    }

    public static Font.PreparedText prepareText(Font font, FormattedCharSequence line, float x, float y, int color,
                                                boolean drawShadow, boolean includeEmpty, int backgroundColor) {
        if (!(line instanceof Line cached) || !cached.isEnabled() || !cached.isCacheable(font)) {
            return font.prepareText(line, x, y, color, drawShadow, includeEmpty, backgroundColor);
        }

        int kind = drawShadow ? Prepared.SHADOWED : Prepared.PLAIN;
        Prepared prepared = cached.find(kind, x, y, color, includeEmpty, backgroundColor);
        if (prepared != null) {
            return prepared.text;
        }

        prepared = new Prepared(kind, x, y, color, includeEmpty, backgroundColor,
                font.prepareText(line, x, y, color, drawShadow, includeEmpty, backgroundColor));
        cached.add(prepared);
        return prepared.text;
    }

    public static Font.PreparedText prepare8xTextOutline(Font font, FormattedCharSequence line, float x, float y, int outlineColor) {
        if (!(line instanceof Line cached) || !cached.isEnabled() || !cached.isCacheable(font)) {
            return font.prepare8xTextOutline(line, x, y, outlineColor);
        }

        Prepared prepared = cached.find(Prepared.OUTLINE, x, y, outlineColor, false, 0);
        if (prepared != null) {
            return prepared.text;
        }

        prepared = new Prepared(Prepared.OUTLINE, x, y, outlineColor, false, 0, font.prepare8xTextOutline(line, x, y, outlineColor));
        cached.add(prepared);
        return prepared.text;
    }

    public static void invalidate() {
        generation++;
    }

    private static final class Line implements FormattedCharSequence {
        private final FormattedCharSequence delegate;
        private final int source;
        private Boolean obfuscated;

        private Font font;
        private int generation = -1;
        private int width;
        private final Prepared[] prepared = new Prepared[VARIANTS];
        private int next;

        Line(FormattedCharSequence delegate, int source) {
            this.delegate = delegate;
            this.source = source;
        }

        boolean isEnabled() {
            return switch (this.source) {
                case NAME_TAG -> Initializer.CONFIG.nameTagTextCache;
                case TEXT_DISPLAY -> Initializer.CONFIG.textDisplayTextCache;
                default -> true;
            };
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

        Prepared find(int kind, float x, float y, int color, boolean includeEmpty, int backgroundColor) {
            for (Prepared prepared : this.prepared) {
                if (prepared == null) {
                    return null;
                }
                if (prepared.matches(kind, x, y, color, includeEmpty, backgroundColor)) {
                    return prepared;
                }
            }
            return null;
        }

        void add(Prepared prepared) {
            this.prepared[this.next] = prepared;
            this.next = (this.next + 1) % VARIANTS;
        }

        private void validate(Font font) {
            if (this.font != font || this.generation != SignTextCache.generation) {
                this.font = font;
                this.generation = SignTextCache.generation;
                this.width = -1;
                java.util.Arrays.fill(this.prepared, null);
                this.next = 0;
            }
        }
    }

    private record Prepared(int kind, float x, float y, int color, boolean includeEmpty, int backgroundColor, Font.PreparedText text) {
        static final int PLAIN = 0, SHADOWED = 1, OUTLINE = 2;

        boolean matches(int kind, float x, float y, int color, boolean includeEmpty, int backgroundColor) {
            return this.kind == kind && Float.compare(this.x, x) == 0 && Float.compare(this.y, y) == 0 && this.color == color
                    && this.includeEmpty == includeEmpty && this.backgroundColor == backgroundColor;
        }
    }
}
