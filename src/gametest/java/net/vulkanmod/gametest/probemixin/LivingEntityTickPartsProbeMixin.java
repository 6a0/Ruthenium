package net.vulkanmod.gametest.probemixin;

import com.llamalad7.mixinextras.injector.wrapmethod.WrapMethod;
import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.mojang.blaze3d.systems.RenderSystem;
import net.minecraft.world.entity.LivingEntity;
import net.vulkanmod.gametest.EntityCostProbe;
import org.spongepowered.asm.mixin.Mixin;

@Mixin(LivingEntity.class)
public class LivingEntityTickPartsProbeMixin {
    @WrapMethod(method = "pushEntities")
    private void timePush(Operation<Void> original) {
        long start = System.nanoTime();
        original.call();
        if (RenderSystem.isOnRenderThread()) {
            EntityCostProbe.TICK_PARTS.add("LivingEntity.pushEntities", System.nanoTime() - start, 0);
        }
    }

    @WrapMethod(method = "baseTick")
    private void timeBaseTick(Operation<Void> original) {
        long start = System.nanoTime();
        original.call();
        if (RenderSystem.isOnRenderThread()) {
            EntityCostProbe.TICK_PARTS.add("LivingEntity.baseTick", System.nanoTime() - start, 0);
        }
    }

    @WrapMethod(method = "aiStep")
    private void timeAiStep(Operation<Void> original) {
        long start = System.nanoTime();
        original.call();
        if (RenderSystem.isOnRenderThread()) {
            EntityCostProbe.TICK_PARTS.add("LivingEntity.aiStep", System.nanoTime() - start, 0);
        }
    }

    @WrapMethod(method = "tickEffects")
    private void timeEffects(Operation<Void> original) {
        long start = System.nanoTime();
        original.call();
        if (RenderSystem.isOnRenderThread()) {
            EntityCostProbe.TICK_PARTS.add("LivingEntity.tickEffects", System.nanoTime() - start, 0);
        }
    }
}
