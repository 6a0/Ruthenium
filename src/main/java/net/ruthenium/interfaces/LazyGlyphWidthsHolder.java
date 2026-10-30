package net.ruthenium.interfaces;

import net.ruthenium.render.text.LazyGlyphWidths;
import org.jspecify.annotations.Nullable;

/** Implemented by {@code FontSet} */
public interface LazyGlyphWidthsHolder {

    /** The lazily built glyphs by width since the last reload, or null when vanilla built them (switch off) */
    @Nullable LazyGlyphWidths ruthenium$lazyGlyphWidths();
}
