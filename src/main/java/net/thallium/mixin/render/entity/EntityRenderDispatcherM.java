package net.thallium.mixin.render.entity;

import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.llamalad7.mixinextras.injector.wrapoperation.WrapOperation;
import net.minecraft.client.renderer.entity.EntityRenderDispatcher;
import net.minecraft.client.renderer.entity.EntityRenderer;
import net.minecraft.world.entity.EntityType;
import net.thallium.interfaces.EntityRendererMap;
import net.thallium.render.entity.EntityRendererCache;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;

import java.util.Map;

/**
 * The renderer map lookups of non-player entities and their render states go through the renderer kept on the entity
 * type ({@link EntityRendererCache}).
 */
@Mixin(EntityRenderDispatcher.class)
public class EntityRenderDispatcherM implements EntityRendererMap {
    @Shadow private Map<EntityType<?>, EntityRenderer<?, ?>> renderers;

    @Override
    public Map<?, ?> thallium$renderers() {
        return this.renderers;
    }

    @WrapOperation(method = {
            "getRenderer(Lnet/minecraft/world/entity/Entity;)Lnet/minecraft/client/renderer/entity/EntityRenderer;",
            "getRenderer(Lnet/minecraft/client/renderer/entity/state/EntityRenderState;)Lnet/minecraft/client/renderer/entity/EntityRenderer;"
    }, at = @At(value = "INVOKE", target = "Ljava/util/Map;get(Ljava/lang/Object;)Ljava/lang/Object;"))
    private Object thallium$cachedRenderer(Map<?, ?> map, Object key, Operation<Object> original) {
        if (!EntityRendererCache.cacheable(map, key)) {
            return original.call(map, key);
        }

        Object renderer = EntityRendererCache.cached(map, key);
        if (renderer == null) {
            renderer = original.call(map, key);
            EntityRendererCache.store(map, key, renderer);
        }
        return renderer;
    }
}
