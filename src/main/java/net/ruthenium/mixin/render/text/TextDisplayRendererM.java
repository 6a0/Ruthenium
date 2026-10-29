package net.ruthenium.mixin.render.text;

import net.minecraft.client.renderer.entity.DisplayRenderer;
import net.minecraft.util.FormattedCharSequence;
import net.ruthenium.render.text.SignTextCache;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.ModifyArg;

/**
 * Text display lines carry their own {@link SignTextCache} entry ({@code textDisplayTextCache}): a text display keeps
 * its split lines ({@code TextDisplay.cacheDisplay}) until its text or line width changes, and each line is wrapped
 * once, when it is split, so its prepared glyphs are reused across frames as sign lines' are.
 */
@Mixin(DisplayRenderer.TextDisplayRenderer.class)
public class TextDisplayRendererM {

    @ModifyArg(method = "splitLines", index = 0, at = @At(value = "INVOKE",
            target = "Lnet/minecraft/world/entity/Display$TextDisplay$CachedLine;<init>(Lnet/minecraft/util/FormattedCharSequence;I)V"))
    private FormattedCharSequence ruthenium$wrapLine(FormattedCharSequence line) {
        return SignTextCache.wrap(line, SignTextCache.TEXT_DISPLAY);
    }
}
