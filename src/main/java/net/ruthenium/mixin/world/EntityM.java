package net.ruthenium.mixin.world;

import com.llamalad7.mixinextras.sugar.Local;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.level.Level;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.Vec3;
import net.minecraft.world.phys.shapes.VoxelShape;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Redirect;

import java.util.List;

/**
 * {@code collide} queries the entities around an entity for their collision shapes before it knows whether it
 * moves; without movement it returns the movement as it is and never reads them (the step up needs a collision).
 * Entities move by zero every tick while they float or rest (dropped items without gravity, experience orbs, boats),
 * so the query is skipped then.
 */
@Mixin(Entity.class)
public class EntityM {
    @Redirect(method = "collide", at = @At(value = "INVOKE",
            target = "Lnet/minecraft/world/level/Level;getEntityCollisions(Lnet/minecraft/world/entity/Entity;Lnet/minecraft/world/phys/AABB;)Ljava/util/List;"))
    private List<VoxelShape> skipUnusedCollisions(Level level, Entity source, AABB testArea, @Local(argsOnly = true) Vec3 movement) {
        return movement.lengthSqr() == 0.0 ? List.of() : level.getEntityCollisions(source, testArea);
    }
}
