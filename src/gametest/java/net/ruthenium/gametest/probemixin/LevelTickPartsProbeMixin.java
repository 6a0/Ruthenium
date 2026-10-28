package net.ruthenium.gametest.probemixin;

import com.llamalad7.mixinextras.injector.wrapmethod.WrapMethod;
import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.mojang.blaze3d.systems.RenderSystem;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.level.Level;
import net.minecraft.world.phys.AABB;
import net.ruthenium.gametest.EntityCostProbe;
import org.spongepowered.asm.mixin.Mixin;

import java.util.List;
import java.util.function.Predicate;

@Mixin(Level.class)
public class LevelTickPartsProbeMixin {
    @WrapMethod(method = "getEntities(Lnet/minecraft/world/entity/Entity;Lnet/minecraft/world/phys/AABB;Ljava/util/function/Predicate;)Ljava/util/List;")
    private List<Entity> timeGetEntities(Entity except, AABB box, Predicate<? super Entity> selector, Operation<List<Entity>> original) {
        long start = System.nanoTime();
        List<Entity> result = original.call(except, box, selector);
        if (RenderSystem.isOnRenderThread()) {
            EntityCostProbe.TICK_PARTS.add("Level.getEntities(except, box, predicate)", System.nanoTime() - start, result.size());
        }
        return result;
    }
}
