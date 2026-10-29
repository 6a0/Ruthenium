package net.ruthenium.mixin.render.text;

import net.minecraft.network.chat.Component;
import net.minecraft.world.entity.Entity;
import net.ruthenium.interfaces.KeptNameTags;
import org.jspecify.annotations.Nullable;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;

/** The name tag components an entity showed last ({@link net.ruthenium.render.text.EntityTextCache}) */
@Mixin(Entity.class)
public class EntityNameTagHolderM implements KeptNameTags {
    @Unique private @Nullable Component ruthenium$nameTag;
    @Unique private @Nullable Component ruthenium$belowName;

    @Override
    public @Nullable Component ruthenium$keptNameTag(boolean belowName) {
        return belowName ? this.ruthenium$belowName : this.ruthenium$nameTag;
    }

    @Override
    public void ruthenium$keepNameTag(boolean belowName, Component component) {
        if (belowName) {
            this.ruthenium$belowName = component;
        } else {
            this.ruthenium$nameTag = component;
        }
    }
}
