package net.ruthenium.mixin.render.text;

import net.minecraft.client.gui.Font;
import net.minecraft.locale.Language;
import net.minecraft.network.chat.MutableComponent;
import net.minecraft.util.FormattedCharSequence;
import net.ruthenium.interfaces.NameTagLayout;
import org.jspecify.annotations.Nullable;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;

/**
 * The layout a name tag component keeps across frames ({@link net.ruthenium.render.text.EntityTextCache}); only set
 * for components shown as name tags.
 */
@Mixin(MutableComponent.class)
public class MutableComponentM implements NameTagLayout {
    @Unique private @Nullable Font ruthenium$widthFont;
    @Unique private @Nullable Language ruthenium$widthLanguage;
    @Unique private int ruthenium$widthGeneration;
    @Unique private int ruthenium$width;
    @Unique private @Nullable FormattedCharSequence ruthenium$line;

    @Override
    public boolean ruthenium$hasWidth(Font font, Language language, int generation) {
        return this.ruthenium$widthFont == font && this.ruthenium$widthLanguage == language && this.ruthenium$widthGeneration == generation;
    }

    @Override
    public int ruthenium$width() {
        return this.ruthenium$width;
    }

    @Override
    public void ruthenium$setWidth(Font font, Language language, int generation, int width) {
        this.ruthenium$widthFont = font;
        this.ruthenium$widthLanguage = language;
        this.ruthenium$widthGeneration = generation;
        this.ruthenium$width = width;
    }

    @Override
    public @Nullable FormattedCharSequence ruthenium$line() {
        return this.ruthenium$line;
    }

    @Override
    public void ruthenium$setLine(FormattedCharSequence line) {
        this.ruthenium$line = line;
    }
}
