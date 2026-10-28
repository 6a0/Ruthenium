package net.rubidium.mixin.render.entity;

import net.minecraft.world.entity.EntityType;
import net.rubidium.interfaces.EntityRendererHolder;
import net.rubidium.render.entity.EntityRendererCache;
import org.jspecify.annotations.Nullable;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;

/** Keeps the type's entity renderer ({@link EntityRendererCache}) */
@Mixin(EntityType.class)
public class EntityTypeM implements EntityRendererHolder {
    @Unique private EntityRendererCache.@Nullable Entry rubidium$rendererEntry;

    @Override
    public EntityRendererCache.@Nullable Entry rubidium$rendererEntry() {
        return this.rubidium$rendererEntry;
    }

    @Override
    public void rubidium$setRendererEntry(EntityRendererCache.Entry entry) {
        this.rubidium$rendererEntry = entry;
    }
}
