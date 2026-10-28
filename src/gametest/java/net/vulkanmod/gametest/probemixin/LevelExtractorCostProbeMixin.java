package net.vulkanmod.gametest.probemixin;

import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.llamalad7.mixinextras.injector.wrapoperation.WrapOperation;
import net.minecraft.client.renderer.culling.Frustum;
import net.minecraft.client.renderer.entity.state.EntityRenderState;
import net.minecraft.client.renderer.extract.LevelExtractor;
import net.minecraft.world.entity.Entity;
import net.vulkanmod.gametest.EntityCostProbe;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;

@Mixin(LevelExtractor.class)
public class LevelExtractorCostProbeMixin {
    @WrapOperation(method = "extractVisibleEntities", at = @At(value = "INVOKE",
            target = "Lnet/minecraft/client/renderer/extract/LevelExtractor;extractEntity(Lnet/minecraft/world/entity/Entity;F)Lnet/minecraft/client/renderer/entity/state/EntityRenderState;"))
    private EntityRenderState timeExtract(LevelExtractor extractor, Entity entity, float partialTicks, Operation<EntityRenderState> original) {
        long start = System.nanoTime();
        EntityRenderState state = original.call(extractor, entity, partialTicks);
        EntityCostProbe.EXTRACT.add(entity.getType(), System.nanoTime() - start, 0);
        return state;
    }

    @WrapOperation(method = "extractVisibleEntities", at = @At(value = "INVOKE",
            target = "Lnet/minecraft/client/renderer/extract/LevelExtractor;isEntityVisible(Lnet/minecraft/world/entity/Entity;Lnet/minecraft/client/renderer/culling/Frustum;DDDFJ)Z"))
    private boolean timeVisible(LevelExtractor extractor, Entity entity, Frustum frustum, double x, double y, double z, float partialTicks,
                                long fade, Operation<Boolean> original) {
        long start = System.nanoTime();
        boolean visible = original.call(extractor, entity, frustum, x, y, z, partialTicks, fade);
        EntityCostProbe.PARTS.add("LevelExtractor.isEntityVisible", System.nanoTime() - start, 0);
        return visible;
    }
}
