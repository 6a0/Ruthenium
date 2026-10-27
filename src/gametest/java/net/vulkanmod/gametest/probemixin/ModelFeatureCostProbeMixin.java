package net.vulkanmod.gametest.probemixin;

import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.llamalad7.mixinextras.injector.wrapoperation.WrapOperation;
import com.mojang.blaze3d.vertex.PoseStack;
import com.mojang.blaze3d.vertex.VertexConsumer;
import net.minecraft.client.model.Model;
import net.minecraft.client.renderer.feature.ModelFeatureRenderer;
import net.vulkanmod.gametest.EntityCostProbe;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;

import java.util.List;

@Mixin(value = ModelFeatureRenderer.class, priority = 1500)
public class ModelFeatureCostProbeMixin {
    @WrapOperation(method = "buildGroup", at = @At(value = "INVOKE",
            target = "Lnet/minecraft/client/renderer/feature/ModelFeatureRenderer;prepareModel(Lnet/minecraft/client/renderer/feature/ModelFeatureRenderer$Submit;)V"))
    private void timeModel(ModelFeatureRenderer renderer, ModelFeatureRenderer.Submit<?> submit, Operation<Void> original) {
        long start = System.nanoTime();
        original.call(renderer, submit);
        EntityCostProbe.MODELS.add(List.of(submit.model().getClass(), submit.renderType().pipeline().getLocation().getPath()),
                System.nanoTime() - start, 0);
    }

    @WrapOperation(method = "prepareModel", at = @At(value = "INVOKE", target = "Lnet/minecraft/client/model/Model;setupAnim(Ljava/lang/Object;)V"))
    private void timeSetupAnim(Model<?> model, Object state, Operation<Void> original) {
        long start = System.nanoTime();
        original.call(model, state);
        EntityCostProbe.SETUP_ANIM.add(model.getClass(), System.nanoTime() - start, 0);
    }

    @WrapOperation(method = "prepareModel", at = @At(value = "INVOKE", target = "Lnet/minecraft/client/model/Model;renderToBuffer(Lcom/mojang/blaze3d/vertex/PoseStack;Lcom/mojang/blaze3d/vertex/VertexConsumer;III)V"))
    private void timeRender(Model<?> model, PoseStack poseStack, VertexConsumer buffer, int light, int overlay, int color, Operation<Void> original) {
        long start = System.nanoTime();
        original.call(model, poseStack, buffer, light, overlay, color);
        EntityCostProbe.MODEL_RENDER.add(model.getClass(), System.nanoTime() - start, 0);
    }
}
