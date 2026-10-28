package net.thallium.interfaces;

import net.thallium.render.entity.EntityRendererCache;
import org.jspecify.annotations.Nullable;

/** An entity type's renderer, kept by {@link EntityRendererCache} on the {@code EntityType} */
public interface EntityRendererHolder {
    EntityRendererCache.@Nullable Entry thallium$rendererEntry();

    void thallium$setRendererEntry(EntityRendererCache.Entry entry);
}
