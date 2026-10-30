package net.ruthenium.render.text;

import com.mojang.blaze3d.font.GlyphProvider;
import com.mojang.blaze3d.font.UnbakedGlyph;
import it.unimi.dsi.fastutil.ints.Int2ObjectMap;
import it.unimi.dsi.fastutil.ints.Int2ObjectOpenHashMap;
import it.unimi.dsi.fastutil.ints.IntArrayList;
import it.unimi.dsi.fastutil.ints.IntList;
import it.unimi.dsi.fastutil.ints.IntOpenHashSet;
import it.unimi.dsi.fastutil.ints.IntSet;
import net.minecraft.client.gui.font.FontOption;
import net.minecraft.client.gui.font.glyphs.SpecialGlyphs;
import net.minecraft.util.Mth;

import java.util.ArrayList;
import java.util.List;
import java.util.Set;

/**
 * Switch {@code lazyGlyphWidths}: a font set's code points by glyph advance ({@code FontSet.glyphsByWidth}), which only
 * obfuscated text reads ({@code FontSet.getRandomGlyph}), built on the first request instead of on every font reload
 * (~1.6 MB of {@code int[]} over vanilla's font sets, mostly the unifont pages of the default and uniform fonts).
 * <p>
 * One instance per font set reload; a reload replaces it, so a build that races a reload can only fill the old one.
 * The table is built under the instance's lock and published through a volatile field, so text laid out on another
 * thread never sees a half-built table. It is built exactly as {@code FontSet.selectProviders} builds vanilla's (same
 * providers, same code point order), so obfuscated text picks the same glyphs from the same random numbers.
 */
public final class LazyGlyphWidths {
    private final List<GlyphProvider.Conditional> providers;
    private final Set<FontOption> options;
    private volatile Int2ObjectMap<IntList> widths;

    public LazyGlyphWidths(List<GlyphProvider.Conditional> providers, Set<FontOption> options) {
        this.providers = providers;
        this.options = options;
    }

    public boolean isBuilt() {
        return this.widths != null;
    }

    public Int2ObjectMap<IntList> widths() {
        Int2ObjectMap<IntList> widths = this.widths;
        if (widths == null) {
            synchronized (this) {
                widths = this.widths;
                if (widths == null) {
                    widths = build();
                    this.widths = widths;
                }
            }
        }
        return widths;
    }

    private Int2ObjectMap<IntList> build() {
        IntSet supportedGlyphs = new IntOpenHashSet();
        List<GlyphProvider> selectedProviders = new ArrayList<>();
        for (GlyphProvider.Conditional conditional : this.providers) {
            if (conditional.filter().apply(this.options)) {
                selectedProviders.add(conditional.provider());
                supportedGlyphs.addAll(conditional.provider().getSupportedGlyphs());
            }
        }

        Int2ObjectMap<IntList> widths = new Int2ObjectOpenHashMap<>();
        supportedGlyphs.forEach(codepoint -> {
            for (GlyphProvider provider : selectedProviders) {
                UnbakedGlyph glyph = provider.getGlyph(codepoint);
                if (glyph != null) {
                    if (glyph.info() != SpecialGlyphs.MISSING) {
                        widths.computeIfAbsent(Mth.ceil(glyph.info().getAdvance(false)), _ -> new IntArrayList()).add(codepoint);
                    }
                    break;
                }
            }
        });
        for (IntList list : widths.values()) {
            ((IntArrayList) list).trim();
        }
        return widths;
    }
}
