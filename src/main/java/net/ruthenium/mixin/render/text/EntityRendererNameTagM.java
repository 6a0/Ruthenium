package net.ruthenium.mixin.render.text;

import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.llamalad7.mixinextras.injector.wrapoperation.WrapOperation;
import net.minecraft.client.renderer.entity.EntityRenderer;
import net.minecraft.network.chat.Component;
import net.minecraft.world.entity.Entity;
import net.ruthenium.render.text.EntityTextCache;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;

/** Name tag components equal to the ones an entity showed last are replaced by those ({@link EntityTextCache}) */
@Mixin(EntityRenderer.class)
public class EntityRendererNameTagM {

    @WrapOperation(method = "extractNameTags(Lnet/minecraft/world/entity/Entity;Lnet/minecraft/client/renderer/entity/state/EntityRenderState;FDD)V",
            at = @At(value = "INVOKE",
                    target = "Lnet/minecraft/client/renderer/entity/EntityRenderer;getNameTag(Lnet/minecraft/world/entity/Entity;)Lnet/minecraft/network/chat/Component;"))
    private Component ruthenium$keepNameTag(EntityRenderer<?, ?> renderer, Entity entity, Operation<Component> original) {
        return EntityTextCache.keep(entity, original.call(renderer, entity), false);
    }

    @WrapOperation(method = "extractNameTags(Lnet/minecraft/world/entity/Entity;Lnet/minecraft/client/renderer/entity/state/EntityRenderState;FDD)V",
            at = @At(value = "INVOKE", target = "Lnet/minecraft/world/entity/Entity;belowNameDisplay()Lnet/minecraft/network/chat/Component;"))
    private Component ruthenium$keepBelowName(Entity entity, Operation<Component> original) {
        return EntityTextCache.keep(entity, original.call(entity), true);
    }
}
