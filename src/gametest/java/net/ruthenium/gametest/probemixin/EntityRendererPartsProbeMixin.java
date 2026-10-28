package net.ruthenium.gametest.probemixin;

import com.llamalad7.mixinextras.injector.wrapmethod.WrapMethod;
import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.mojang.blaze3d.vertex.PoseStack;
import net.minecraft.client.renderer.SubmitNodeCollector;
import net.minecraft.client.renderer.entity.EntityRenderer;
import net.minecraft.client.renderer.entity.state.EntityRenderState;
import net.minecraft.client.renderer.state.level.CameraRenderState;
import net.minecraft.network.chat.Component;
import net.minecraft.world.entity.Entity;
import net.ruthenium.gametest.EntityCostProbe;
import org.spongepowered.asm.mixin.Mixin;

@Mixin(EntityRenderer.class)
public class EntityRendererPartsProbeMixin {
    @WrapMethod(method = "getPackedLightCoords")
    private int timeLight(Entity entity, float partialTicks, Operation<Integer> original) {
        long start = System.nanoTime();
        int light = original.call(entity, partialTicks);
        EntityCostProbe.PARTS.add("EntityRenderer.getPackedLightCoords", System.nanoTime() - start, 0);
        return light;
    }

    @WrapMethod(method = "finalizeRenderState")
    private void timeFinalize(Entity entity, EntityRenderState state, Operation<Void> original) {
        long start = System.nanoTime();
        original.call(entity, state);
        EntityCostProbe.PARTS.add("EntityRenderer.finalizeRenderState", System.nanoTime() - start, 0);
    }

    @WrapMethod(method = "getNameTag")
    private Component timeNameTag(Entity entity, Operation<Component> original) {
        long start = System.nanoTime();
        Component name = original.call(entity);
        EntityCostProbe.PARTS.add("EntityRenderer.getNameTag", System.nanoTime() - start, 0);
        return name;
    }

    @WrapMethod(method = "submitNameDisplay(Lnet/minecraft/client/renderer/entity/state/EntityRenderState;Lcom/mojang/blaze3d/vertex/PoseStack;Lnet/minecraft/client/renderer/SubmitNodeCollector;Lnet/minecraft/client/renderer/state/level/CameraRenderState;I)V")
    private void timeSubmitName(EntityRenderState state, PoseStack poseStack, SubmitNodeCollector collector, CameraRenderState camera, int offset,
                                Operation<Void> original) {
        long start = System.nanoTime();
        original.call(state, poseStack, collector, camera, offset);
        EntityCostProbe.PARTS.add("EntityRenderer.submitNameDisplay" + (state.nameTag != null ? " (named)" : ""), System.nanoTime() - start, 0);
    }
}
