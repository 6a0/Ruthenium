package net.vulkanmod.interfaces;

import net.vulkanmod.render.entity.EntityRendererCache;
import org.jspecify.annotations.Nullable;

/** An entity type's renderer, kept by {@link EntityRendererCache} on the {@code EntityType} */
public interface EntityRendererHolder {
    EntityRendererCache.@Nullable Entry vulkanmod$rendererEntry();

    void vulkanmod$setRendererEntry(EntityRendererCache.Entry entry);
}
