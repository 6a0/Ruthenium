package net.thallium.mixin.render.entity;

import net.minecraft.world.entity.EntityType;
import net.thallium.interfaces.EntityRendererHolder;
import net.thallium.render.entity.EntityRendererCache;
import org.jspecify.annotations.Nullable;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;

/** Keeps the type's entity renderer ({@link EntityRendererCache}) */
@Mixin(EntityType.class)
public class EntityTypeM implements EntityRendererHolder {
    @Unique private EntityRendererCache.@Nullable Entry thallium$rendererEntry;

    @Override
    public EntityRendererCache.@Nullable Entry thallium$rendererEntry() {
        return this.thallium$rendererEntry;
    }

    @Override
    public void thallium$setRendererEntry(EntityRendererCache.Entry entry) {
        this.thallium$rendererEntry = entry;
    }
}
