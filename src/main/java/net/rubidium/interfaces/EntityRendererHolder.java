package net.rubidium.interfaces;

import net.rubidium.render.entity.EntityRendererCache;
import org.jspecify.annotations.Nullable;

/** An entity type's renderer, kept by {@link EntityRendererCache} on the {@code EntityType} */
public interface EntityRendererHolder {
    EntityRendererCache.@Nullable Entry rubidium$rendererEntry();

    void rubidium$setRendererEntry(EntityRendererCache.Entry entry);
}
