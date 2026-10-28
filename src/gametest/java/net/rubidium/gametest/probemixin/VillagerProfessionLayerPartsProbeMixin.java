package net.rubidium.gametest.probemixin;

import com.llamalad7.mixinextras.injector.wrapmethod.WrapMethod;
import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.mojang.blaze3d.vertex.PoseStack;
import net.minecraft.client.renderer.SubmitNodeCollector;
import net.minecraft.client.renderer.entity.layers.VillagerProfessionLayer;
import net.minecraft.client.renderer.entity.state.LivingEntityRenderState;
import net.rubidium.gametest.EntityCostProbe;
import org.spongepowered.asm.mixin.Mixin;

@Mixin(VillagerProfessionLayer.class)
public class VillagerProfessionLayerPartsProbeMixin {
    @WrapMethod(method = "submit(Lcom/mojang/blaze3d/vertex/PoseStack;Lnet/minecraft/client/renderer/SubmitNodeCollector;ILnet/minecraft/client/renderer/entity/state/LivingEntityRenderState;FF)V")
    private void timeSubmit(PoseStack poseStack, SubmitNodeCollector collector, int light, LivingEntityRenderState state, float yRot, float xRot,
                            Operation<Void> original) {
        long start = System.nanoTime();
        original.call(poseStack, collector, light, state, yRot, xRot);
        EntityCostProbe.PARTS.add("VillagerProfessionLayer.submit", System.nanoTime() - start, 0);
    }
}
