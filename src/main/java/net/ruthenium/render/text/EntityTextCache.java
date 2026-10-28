package net.ruthenium.render.text;

import com.mojang.blaze3d.systems.RenderSystem;
import net.minecraft.client.gui.Font;
import net.minecraft.locale.Language;
import net.minecraft.network.chat.Component;
import net.minecraft.network.chat.FormattedText;
import net.minecraft.util.FormattedCharSequence;
import net.minecraft.world.entity.Entity;
import net.ruthenium.Initializer;
import net.ruthenium.interfaces.KeptNameTags;
import net.ruthenium.interfaces.NameTagLayout;
import org.jspecify.annotations.Nullable;

/**
 * Name tags keep their text layout across frames ({@code nameTagTextCache}).
 * <p>
 * Vanilla builds an entity's name tag anew every frame ({@code Entity.getDisplayName}: a new component with the team
 * formatting, hover event and insertion; for players also a click event), and then lays it out three times: its width
 * ({@code Font.width}), its visual order ({@code Component.getVisualOrderText}, which runs ICU's bidirectional
 * reordering and Arabic shaping over the whole text) and the glyphs of each of its two submits
 * ({@code Font.prepareText}: the see-through pass with a background and the normal pass). The component caches its
 * visual order, but the component is new each frame. The line below the name ({@code belowNameDisplay}, a score or a
 * mannequin's description) is built the same way.
 * <p>
 * Here the entity keeps the component it showed last ({@link KeptNameTags}); a new one that is equal to it (deep
 * equality: contents, style with its events, siblings) is replaced by the kept one, so the kept component's own
 * visual order cache applies, and the component carries its width and a {@link SignTextCache} line around its visual
 * order ({@link NameTagLayout}), whose prepared glyphs are reused. Equal components lay out the same, so the result is
 * the same: the width is kept per font, language and font generation, the visual order by vanilla per language, the
 * glyphs per font generation and parameters (obfuscated text is laid out anew each time, as in vanilla). Render thread
 * only.
 */
public final class EntityTextCache {
    private EntityTextCache() {
    }

    /** The name tag or below-name component to show: the kept one if equal, else {@code component} (which is kept) */
    public static @Nullable Component keep(Entity entity, @Nullable Component component, boolean belowName) {
        if (component == null || !Initializer.CONFIG.nameTagTextCache || !RenderSystem.isOnRenderThread()) {
            return component;
        }
        KeptNameTags holder = (KeptNameTags) entity;
        Component kept = holder.ruthenium$keptNameTag(belowName);
        if (kept == component) {
            return component;
        }
        if (kept != null && kept.equals(component)) {
            return kept;
        }
        holder.ruthenium$keepNameTag(belowName, component);
        return component;
    }

    /** The {@code Font.width} of a name tag kept on its component, or -1 */
    public static int keptWidth(Font font, FormattedText text) {
        if (!Initializer.CONFIG.nameTagTextCache || !(text instanceof NameTagLayout layout) || !RenderSystem.isOnRenderThread()
                || !layout.ruthenium$hasWidth(font, Language.getInstance(), SignTextCache.generation())) {
            return -1;
        }
        return layout.ruthenium$width();
    }

    public static void keepWidth(Font font, FormattedText text, int width) {
        if (Initializer.CONFIG.nameTagTextCache && text instanceof NameTagLayout layout && width >= 0 && RenderSystem.isOnRenderThread()) {
            layout.ruthenium$setWidth(font, Language.getInstance(), SignTextCache.generation(), width);
        }
    }

    /**
     * {@code Component.getVisualOrderText} of a name tag ({@code sequence}), wrapped once in a {@link SignTextCache}
     * line that is kept on the component as long as the component returns the same sequence (vanilla recomputes it
     * when the language changes)
     */
    public static FormattedCharSequence visualOrder(Component component, FormattedCharSequence sequence) {
        if (!Initializer.CONFIG.nameTagTextCache || !(component instanceof NameTagLayout layout) || !RenderSystem.isOnRenderThread()) {
            return sequence;
        }
        FormattedCharSequence line = layout.ruthenium$line();
        if (line == null || SignTextCache.unwrap(line) != sequence) {
            line = SignTextCache.wrap(sequence, SignTextCache.NAME_TAG);
            layout.ruthenium$setLine(line);
        }
        return line;
    }
}
