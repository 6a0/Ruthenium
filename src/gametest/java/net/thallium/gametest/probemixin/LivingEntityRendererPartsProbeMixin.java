package net.thallium.gametest.probemixin;

import com.llamalad7.mixinextras.injector.wrapmethod.WrapMethod;
import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import net.minecraft.client.renderer.entity.LivingEntityRenderer;
import net.minecraft.client.renderer.entity.state.LivingEntityRenderState;
import net.minecraft.client.renderer.rendertype.RenderType;
import net.minecraft.world.entity.LivingEntity;
import net.thallium.gametest.EntityCostProbe;
import org.spongepowered.asm.mixin.Mixin;

@Mixin(LivingEntityRenderer.class)
public class LivingEntityRendererPartsProbeMixin {
    @WrapMethod(method = "extractNameTags(Lnet/minecraft/world/entity/LivingEntity;Lnet/minecraft/client/renderer/entity/state/LivingEntityRenderState;F)V")
    private void timeNameTags(LivingEntity entity, LivingEntityRenderState state, float partialTicks, Operation<Void> original) {
        long start = System.nanoTime();
        original.call(entity, state, partialTicks);
        EntityCostProbe.PARTS.add("LivingEntityRenderer.extractNameTags", System.nanoTime() - start, 0);
    }

    @WrapMethod(method = "shouldShowName(Lnet/minecraft/world/entity/LivingEntity;D)Z")
    private boolean timeShouldShowName(LivingEntity entity, double distanceSq, Operation<Boolean> original) {
        long start = System.nanoTime();
        boolean show = original.call(entity, distanceSq);
        EntityCostProbe.PARTS.add("LivingEntityRenderer.shouldShowName", System.nanoTime() - start, 0);
        return show;
    }

    @WrapMethod(method = "getRenderType")
    private RenderType timeRenderType(LivingEntityRenderState state, boolean bodyVisible, boolean forceTransparent, boolean glowing,
                                      Operation<RenderType> original) {
        long start = System.nanoTime();
        RenderType type = original.call(state, bodyVisible, forceTransparent, glowing);
        EntityCostProbe.PARTS.add("LivingEntityRenderer.getRenderType", System.nanoTime() - start, 0);
        return type;
    }
}
