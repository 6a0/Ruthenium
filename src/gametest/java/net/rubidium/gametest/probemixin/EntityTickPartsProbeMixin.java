package net.rubidium.gametest.probemixin;

import com.llamalad7.mixinextras.injector.wrapmethod.WrapMethod;
import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.mojang.blaze3d.systems.RenderSystem;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.MoverType;
import net.minecraft.world.phys.Vec3;
import net.rubidium.gametest.EntityCostProbe;
import org.spongepowered.asm.mixin.Mixin;

/** Parts of the client's entity tick (render thread only; the integrated server ticks the same classes) */
@Mixin(Entity.class)
public class EntityTickPartsProbeMixin {
    @WrapMethod(method = "baseTick")
    private void timeBaseTick(Operation<Void> original) {
        long start = System.nanoTime();
        original.call();
        if (RenderSystem.isOnRenderThread()) {
            EntityCostProbe.TICK_PARTS.add("Entity.baseTick", System.nanoTime() - start, 0);
        }
    }

    @WrapMethod(method = "move")
    private void timeMove(MoverType type, Vec3 delta, Operation<Void> original) {
        long start = System.nanoTime();
        original.call(type, delta);
        if (RenderSystem.isOnRenderThread()) {
            EntityCostProbe.TICK_PARTS.add("Entity.move", System.nanoTime() - start, 0);
        }
    }

    @WrapMethod(method = "recordMovement")
    private void timeRecordMovement(MoverType type, Vec3 delta, Operation<Void> original) {
        long start = System.nanoTime();
        original.call(type, delta);
        if (RenderSystem.isOnRenderThread()) {
            EntityCostProbe.TICK_PARTS.add("Entity.recordMovement", System.nanoTime() - start, 0);
        }
    }

    @WrapMethod(method = "updateFluidInteraction")
    private boolean timeFluids(Operation<Boolean> original) {
        long start = System.nanoTime();
        boolean result = original.call();
        if (RenderSystem.isOnRenderThread()) {
            EntityCostProbe.TICK_PARTS.add("Entity.updateFluidInteraction", System.nanoTime() - start, 0);
        }
        return result;
    }

    @WrapMethod(method = "applyEffectsFromBlocks()V")
    private void timeBlockEffects(Operation<Void> original) {
        long start = System.nanoTime();
        original.call();
        if (RenderSystem.isOnRenderThread()) {
            EntityCostProbe.TICK_PARTS.add("Entity.applyEffectsFromBlocks", System.nanoTime() - start, 0);
        }
    }

    @WrapMethod(method = "commonTick")
    private void timeCommonTick(Operation<Void> original) {
        long start = System.nanoTime();
        original.call();
        if (RenderSystem.isOnRenderThread()) {
            EntityCostProbe.TICK_PARTS.add("Entity.commonTick", System.nanoTime() - start, 0);
        }
    }
}
