package net.ruthenium.mixin.world;

import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.llamalad7.mixinextras.injector.wrapoperation.WrapOperation;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.level.Level;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.Vec3;
import net.minecraft.world.phys.shapes.VoxelShape;
import net.ruthenium.Initializer;
import net.ruthenium.world.RestingCollisions;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;

import java.util.List;

/**
 * Keeps the last block collision answer of an entity's move on the client, see {@link RestingCollisions}. The state
 * lives in a separate object created on first use.
 */
@Mixin(Entity.class)
public abstract class EntityRestingCollisionsM {
    @Unique private RestingCollisions.Kept keptCollision;

    @WrapOperation(method = "collide", at = @At(value = "INVOKE",
            target = "Lnet/minecraft/world/entity/Entity;collideBoundingBox(Lnet/minecraft/world/entity/Entity;Lnet/minecraft/world/phys/Vec3;Lnet/minecraft/world/phys/AABB;Lnet/minecraft/world/level/Level;Ljava/util/List;)Lnet/minecraft/world/phys/Vec3;"))
    private Vec3 keepBlockCollisions(Entity source, Vec3 movement, AABB box, Level level, List<VoxelShape> entityColliders,
                                     Operation<Vec3> original) {
        if (!Initializer.CONFIG.fastRestingCollisions || !level.isClientSide() || !entityColliders.isEmpty()) {
            return original.call(source, movement, box, level, entityColliders);
        }
        RestingCollisions.Kept kept = this.keptCollision;
        if (kept != null) {
            Vec3 result = kept.find(level, source, box, movement);
            if (result != null) {
                return result;
            }
        }
        Vec3 result = original.call(source, movement, box, level, entityColliders);
        if (kept == null) {
            kept = this.keptCollision = new RestingCollisions.Kept();
        }
        kept.store(level, source, box, movement, result);
        return result;
    }
}
