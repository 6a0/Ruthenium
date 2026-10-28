package net.thallium.gametest.probemixin;

import com.llamalad7.mixinextras.injector.wrapmethod.WrapMethod;
import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.mojang.blaze3d.vertex.PoseStack;
import net.minecraft.client.renderer.SubmitNodeCollector;
import net.minecraft.client.renderer.entity.layers.HumanoidArmorLayer;
import net.minecraft.client.renderer.entity.layers.ItemInHandLayer;
import net.minecraft.client.renderer.entity.state.EntityRenderState;
import net.thallium.gametest.EntityCostProbe;
import org.spongepowered.asm.mixin.Mixin;

@Mixin({HumanoidArmorLayer.class, ItemInHandLayer.class})
public class LayerPartsProbeMixin {
    @WrapMethod(method = "submit(Lcom/mojang/blaze3d/vertex/PoseStack;Lnet/minecraft/client/renderer/SubmitNodeCollector;ILnet/minecraft/client/renderer/entity/state/EntityRenderState;FF)V")
    private void timeLayer(PoseStack poseStack, SubmitNodeCollector collector, int light, EntityRenderState state, float yRot, float xRot,
                           Operation<Void> original) {
        long start = System.nanoTime();
        original.call(poseStack, collector, light, state, yRot, xRot);
        EntityCostProbe.PARTS.add(this.getClass().getSimpleName() + ".submit", System.nanoTime() - start, 0);
    }
}
