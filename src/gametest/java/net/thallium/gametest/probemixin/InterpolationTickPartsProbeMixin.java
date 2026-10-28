package net.thallium.gametest.probemixin;

import com.llamalad7.mixinextras.injector.wrapmethod.WrapMethod;
import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.mojang.blaze3d.systems.RenderSystem;
import net.minecraft.world.entity.AbstractInterpolationHandler;
import net.minecraft.world.phys.Vec3;
import net.thallium.gametest.EntityCostProbe;
import org.spongepowered.asm.mixin.Mixin;

@Mixin(AbstractInterpolationHandler.class)
public class InterpolationTickPartsProbeMixin {
    @WrapMethod(method = "applyPredictedMovement")
    private void timePredicted(Vec3 delta, Operation<Void> original) {
        long start = System.nanoTime();
        original.call(delta);
        if (RenderSystem.isOnRenderThread()) {
            EntityCostProbe.TICK_PARTS.add("AbstractInterpolationHandler.applyPredictedMovement" + (delta.lengthSqr() == 0 ? " (zero)" : ""),
                    System.nanoTime() - start, 0);
        }
    }
}
