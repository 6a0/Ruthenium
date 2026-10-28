package net.ruthenium.gametest.probemixin;

import com.llamalad7.mixinextras.injector.wrapmethod.WrapMethod;
import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.world.entity.Entity;
import net.ruthenium.gametest.EntityCostProbe;
import org.spongepowered.asm.mixin.Mixin;

@Mixin(ClientLevel.class)
public class ClientLevelCostProbeMixin {
    @WrapMethod(method = "tickNonPassenger")
    private void timeTick(Entity entity, Operation<Void> original) {
        long start = System.nanoTime();
        original.call(entity);
        EntityCostProbe.TICK.add(entity.getType(), System.nanoTime() - start, 0);
    }
}
