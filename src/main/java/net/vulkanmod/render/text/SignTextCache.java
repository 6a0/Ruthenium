package net.vulkanmod.render.text;

import it.unimi.dsi.fastutil.objects.Reference2ObjectOpenHashMap;
import net.minecraft.client.gui.Font;
import net.minecraft.util.FormattedCharSequence;

/**
 * Caches the width and prepared glyphs of sign text lines across frames.
 * <p>
 * Vanilla measures every sign line in {@code AbstractSignRenderer.submitSignText} and lays out its glyphs again
 * in {@code TextFeatureRenderer.renderText}, every frame. Sign lines are {@link FormattedCharSequence}s that
 * {@code SignText} keeps until the text changes, so they can be keyed by identity. Only sequences registered by
 * the sign renderer are cached; other text (name tags etc.) may be built from mutable state.
 * <p>
 * A {@link Font.PreparedText} only depends on the sequence, its parameters and the font's glyphs, and visiting it
 * does not change it. Lines with obfuscated characters pick random glyphs on every layout, so they are not cached.
 * Everything is dropped when a font set reloads its glyphs. Only used from the render thread.
 */
public final class SignTextCache {
    private static final int MAX_ENTRIES = 16384;

    private static final Reference2ObjectOpenHashMap<FormattedCharSequence, Entry> ENTRIES = new Reference2ObjectOpenHashMap<>();

    private SignTextCache() {
    }

    public static int width(Font font, FormattedCharSequence line) {
        // Shared by every empty line, at different positions
        if (line == FormattedCharSequence.EMPTY) {
            return font.width(line);
        }

        Entry entry = ENTRIES.get(line);
        if (entry == null || entry.font != font) {
            if (ENTRIES.size() >= MAX_ENTRIES) {
                ENTRIES.clear();
            }

            entry = new Entry(font, font.width(line), isObfuscated(line));
            ENTRIES.put(line, entry);
        }

        return entry.width;
    }

    public static Font.PreparedText prepareText(Font font, FormattedCharSequence line, float x, float y, int color,
                                                boolean drawShadow, boolean includeEmpty, int backgroundColor) {
        Entry entry = ENTRIES.isEmpty() ? null : ENTRIES.get(line);
        if (entry == null || entry.font != font || entry.obfuscated) {
            return font.prepareText(line, x, y, color, drawShadow, includeEmpty, backgroundColor);
        }

        Prepared prepared = drawShadow ? entry.shadowed : entry.plain;
        if (prepared != null && prepared.matches(x, y, color, includeEmpty, backgroundColor)) {
            return prepared.text;
        }

        prepared = new Prepared(x, y, color, includeEmpty, backgroundColor,
                font.prepareText(line, x, y, color, drawShadow, includeEmpty, backgroundColor));
        if (drawShadow) {
            entry.shadowed = prepared;
        } else {
            entry.plain = prepared;
        }
        return prepared.text;
    }

    public static Font.PreparedText prepare8xTextOutline(Font font, FormattedCharSequence line, float x, float y, int outlineColor) {
        Entry entry = ENTRIES.isEmpty() ? null : ENTRIES.get(line);
        if (entry == null || entry.font != font || entry.obfuscated) {
            return font.prepare8xTextOutline(line, x, y, outlineColor);
        }

        Prepared prepared = entry.outline;
        if (prepared != null && prepared.matches(x, y, outlineColor, false, 0)) {
            return prepared.text;
        }

        prepared = new Prepared(x, y, outlineColor, false, 0, font.prepare8xTextOutline(line, x, y, outlineColor));
        entry.outline = prepared;
        return prepared.text;
    }

    public static void invalidate() {
        ENTRIES.clear();
    }

    private static boolean isObfuscated(FormattedCharSequence line) {
        boolean[] obfuscated = new boolean[1];
        line.accept((position, style, codepoint) -> {
            if (style.isObfuscated()) {
                obfuscated[0] = true;
                return false;
            }
            return true;
        });
        return obfuscated[0];
    }

    private static final class Entry {
        final Font font;
        final int width;
        final boolean obfuscated;
        Prepared plain;
        Prepared shadowed;
        Prepared outline;

        Entry(Font font, int width, boolean obfuscated) {
            this.font = font;
            this.width = width;
            this.obfuscated = obfuscated;
        }
    }

    private record Prepared(float x, float y, int color, boolean includeEmpty, int backgroundColor, Font.PreparedText text) {
        boolean matches(float x, float y, int color, boolean includeEmpty, int backgroundColor) {
            return Float.compare(this.x, x) == 0 && Float.compare(this.y, y) == 0 && this.color == color
                    && this.includeEmpty == includeEmpty && this.backgroundColor == backgroundColor;
        }
    }
}
