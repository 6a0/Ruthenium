package net.ruthenium.mixin.render.entity;

import net.minecraft.world.entity.EntityType;
import net.ruthenium.interfaces.EntityRendererHolder;
import net.ruthenium.render.entity.EntityRendererCache;
import org.jspecify.annotations.Nullable;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;

/** Keeps the type's entity renderer ({@link EntityRendererCache}) */
@Mixin(EntityType.class)
public class EntityTypeM implements EntityRendererHolder {
    @Unique private EntityRendererCache.@Nullable Entry ruthenium$rendererEntry;

    @Override
    public EntityRendererCache.@Nullable Entry ruthenium$rendererEntry() {
        return this.ruthenium$rendererEntry;
    }

    @Override
    public void ruthenium$setRendererEntry(EntityRendererCache.Entry entry) {
        this.ruthenium$rendererEntry = entry;
    }
}
