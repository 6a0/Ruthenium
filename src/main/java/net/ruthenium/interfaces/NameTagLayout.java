package net.ruthenium.interfaces;

import net.minecraft.client.gui.Font;
import net.minecraft.locale.Language;
import net.minecraft.util.FormattedCharSequence;
import org.jspecify.annotations.Nullable;

/** The layout a name tag component keeps ({@link net.ruthenium.render.text.EntityTextCache}) */
public interface NameTagLayout {
    boolean ruthenium$hasWidth(Font font, Language language, int generation);

    int ruthenium$width();

    void ruthenium$setWidth(Font font, Language language, int generation, int width);

    /** The {@link net.ruthenium.render.text.SignTextCache} line around the component's visual order, or null */
    @Nullable FormattedCharSequence ruthenium$line();

    void ruthenium$setLine(FormattedCharSequence line);
}
