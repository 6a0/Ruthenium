package net.vulkanmod.mixin.render.entity;

import net.minecraft.world.entity.EntityType;
import net.vulkanmod.interfaces.EntityRendererHolder;
import net.vulkanmod.render.entity.EntityRendererCache;
import org.jspecify.annotations.Nullable;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;

/** Keeps the type's entity renderer ({@link EntityRendererCache}) */
@Mixin(EntityType.class)
public class EntityTypeM implements EntityRendererHolder {
    @Unique private EntityRendererCache.@Nullable Entry vulkanmod$rendererEntry;

    @Override
    public EntityRendererCache.@Nullable Entry vulkanmod$rendererEntry() {
        return this.vulkanmod$rendererEntry;
    }

    @Override
    public void vulkanmod$setRendererEntry(EntityRendererCache.Entry entry) {
        this.vulkanmod$rendererEntry = entry;
    }
}
