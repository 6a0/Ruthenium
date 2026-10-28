package net.thallium.gametest.probemixin;

import com.llamalad7.mixinextras.injector.wrapmethod.WrapMethod;
import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.mojang.blaze3d.systems.RenderSystem;
import net.minecraft.world.entity.vehicle.boat.AbstractBoat;
import net.thallium.gametest.EntityCostProbe;
import org.spongepowered.asm.mixin.Mixin;

@Mixin(AbstractBoat.class)
public class BoatTickPartsProbeMixin {
    @WrapMethod(method = "getStatus")
    private AbstractBoat.Status timeStatus(Operation<AbstractBoat.Status> original) {
        long start = System.nanoTime();
        AbstractBoat.Status status = original.call();
        if (RenderSystem.isOnRenderThread()) {
            EntityCostProbe.TICK_PARTS.add("AbstractBoat.getStatus", System.nanoTime() - start, 0);
        }
        return status;
    }
}
