package net.ruthenium.interfaces;

import net.ruthenium.render.entity.EntityRendererCache;
import org.jspecify.annotations.Nullable;

/** An entity type's renderer, kept by {@link EntityRendererCache} on the {@code EntityType} */
public interface EntityRendererHolder {
    EntityRendererCache.@Nullable Entry ruthenium$rendererEntry();

    void ruthenium$setRendererEntry(EntityRendererCache.Entry entry);
}
