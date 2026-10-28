package net.ruthenium.gametest.probemixin;

import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.llamalad7.mixinextras.injector.wrapoperation.WrapOperation;
import net.minecraft.client.renderer.feature.FeatureFrameContext;
import net.minecraft.client.renderer.feature.FeatureRenderer;
import net.ruthenium.gametest.EntityCostProbe;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;

import java.util.List;

@Mixin(targets = "net.minecraft.client.renderer.feature.FeatureRenderDispatcher$PreparedGroup")
public class FeatureGroupCostProbeMixin {
    @WrapOperation(method = "prepare", at = @At(value = "INVOKE",
            target = "Lnet/minecraft/client/renderer/feature/FeatureRenderer;prepareGroup(Lnet/minecraft/client/renderer/feature/FeatureFrameContext;Ljava/util/List;Z)V"))
    private void timeGroup(FeatureRenderer<?> renderer, FeatureFrameContext context, List<?> submits, boolean strictlyOrdered, Operation<Void> original) {
        long start = System.nanoTime();
        original.call(renderer, context, submits, strictlyOrdered);
        EntityCostProbe.FEATURE_GROUPS.add(renderer.getClass(), System.nanoTime() - start, submits.size());
    }
}
