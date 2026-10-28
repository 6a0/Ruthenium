package net.ruthenium.gametest.probemixin;

import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.llamalad7.mixinextras.injector.wrapoperation.WrapOperation;
import com.mojang.blaze3d.systems.RenderSystem;
import net.minecraft.world.entity.ExperienceOrb;
import net.minecraft.world.level.Level;
import net.minecraft.world.phys.AABB;
import net.ruthenium.gametest.EntityCostProbe;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;

@Mixin(ExperienceOrb.class)
public class OrbTickPartsProbeMixin {
    @WrapOperation(method = "tick", at = @At(value = "INVOKE", target = "Lnet/minecraft/world/level/Level;noCollision(Lnet/minecraft/world/phys/AABB;)Z"))
    private boolean timeNoCollision(Level level, AABB box, Operation<Boolean> original) {
        long start = System.nanoTime();
        boolean result = original.call(level, box);
        if (RenderSystem.isOnRenderThread()) {
            EntityCostProbe.TICK_PARTS.add("ExperienceOrb.tick noCollision", System.nanoTime() - start, 0);
        }
        return result;
    }
}
