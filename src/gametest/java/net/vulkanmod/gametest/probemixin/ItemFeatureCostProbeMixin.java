package net.vulkanmod.gametest.probemixin;

import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.llamalad7.mixinextras.injector.wrapoperation.WrapOperation;
import net.minecraft.client.renderer.feature.ItemFeatureRenderer;
import net.vulkanmod.gametest.EntityCostProbe;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;

@Mixin(ItemFeatureRenderer.class)
public class ItemFeatureCostProbeMixin {
    @WrapOperation(method = "buildGroup", at = @At(value = "INVOKE",
            target = "Lnet/minecraft/client/renderer/feature/ItemFeatureRenderer;prepareSubmit(Lnet/minecraft/client/renderer/feature/ItemFeatureRenderer$Submit;)V"))
    private void timeItem(ItemFeatureRenderer renderer, ItemFeatureRenderer.Submit submit, Operation<Void> original) {
        long start = System.nanoTime();
        original.call(renderer, submit);
        EntityCostProbe.ITEMS.add(submit.displayContext(), System.nanoTime() - start, submit.quads().size());
    }
}
