package net.thallium.gametest.probemixin;

import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.llamalad7.mixinextras.injector.wrapoperation.WrapOperation;
import net.minecraft.client.renderer.entity.state.EntityRenderState;
import net.minecraft.client.renderer.extract.LevelExtractor;
import net.minecraft.world.entity.Entity;
import net.thallium.gametest.EntityCostProbe;
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
}
