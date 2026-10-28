package net.ruthenium.interfaces;

import net.minecraft.network.chat.Component;
import org.jspecify.annotations.Nullable;

/** The name tag and below-name components an entity showed last ({@link net.ruthenium.render.text.EntityTextCache}) */
public interface KeptNameTags {
    @Nullable Component ruthenium$keptNameTag(boolean belowName);

    void ruthenium$keepNameTag(boolean belowName, Component component);
}
