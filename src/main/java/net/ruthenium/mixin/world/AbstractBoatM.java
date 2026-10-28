package net.ruthenium.mixin.world;

import net.minecraft.core.Direction;
import net.minecraft.core.Vec3i;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.vehicle.boat.AbstractBoat;
import net.minecraft.world.level.Level;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.shapes.BooleanOp;
import net.minecraft.world.phys.shapes.Shapes;
import net.minecraft.world.phys.shapes.VoxelShape;
import net.ruthenium.Initializer;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Redirect;

import java.util.List;
import java.util.function.Predicate;

/**
 * Boats tick on the client every tick even at rest (~9 us each):
 * <ul>
 * <li>{@code getGroundFriction} (their status) tests ~40 blocks around the bottom of the boat, each by moving the
 * block's collision shape to its position and joining it with the boat's bottom slice. For full cubes the join is
 * decided from the boxes: separated by more than 1e-6 on an axis means no overlap (vanilla's own early out uses 1e-7),
 * overlapping by more than 1e-6 on every axis means both shapes fill a common cell. Anything in between takes
 * vanilla's join.</li>
 * <li>Their push search for entities {@code EntitySelector.pushableBy} accepts: on the client that predicate only
 * accepts local players, so the client asks {@code ClientLevel.getPushableEntities}, which tests the local player
 * alone, as vanilla's own living entity push does.</li>
 * </ul>
 */
@Mixin(AbstractBoat.class)
public abstract class AbstractBoatM {
    @Unique private boolean pendingCube;
    @Unique private int cubeX, cubeY, cubeZ;

    @Redirect(method = "getGroundFriction", at = @At(value = "INVOKE",
            target = "Lnet/minecraft/world/phys/shapes/VoxelShape;move(Lnet/minecraft/core/Vec3i;)Lnet/minecraft/world/phys/shapes/VoxelShape;"))
    private VoxelShape deferCubeMove(VoxelShape shape, Vec3i pos) {
        if (shape == Shapes.block() && Initializer.CONFIG.fastBoatTicks) {
            this.pendingCube = true;
            this.cubeX = pos.getX();
            this.cubeY = pos.getY();
            this.cubeZ = pos.getZ();
            return shape;
        }
        this.pendingCube = false;
        return shape.move(pos);
    }

    @Redirect(method = "getGroundFriction", at = @At(value = "INVOKE",
            target = "Lnet/minecraft/world/phys/shapes/Shapes;joinIsNotEmpty(Lnet/minecraft/world/phys/shapes/VoxelShape;Lnet/minecraft/world/phys/shapes/VoxelShape;Lnet/minecraft/world/phys/shapes/BooleanOp;)Z"))
    private boolean joinCube(VoxelShape first, VoxelShape second, BooleanOp op) {
        if (!this.pendingCube) {
            return Shapes.joinIsNotEmpty(first, second, op);
        }
        this.pendingCube = false;
        if (second.isEmpty()) {
            return false;
        }
        boolean overlapsAll = true;
        for (int i = 0; i < 3; i++) {
            Direction.Axis axis = i == 0 ? Direction.Axis.X : i == 1 ? Direction.Axis.Y : Direction.Axis.Z;
            int block = i == 0 ? this.cubeX : i == 1 ? this.cubeY : this.cubeZ;
            double overlap = Math.min(second.max(axis), block + 1.0) - Math.max(second.min(axis), block);
            if (overlap < -1.0E-6) {
                return false;
            }
            overlapsAll &= overlap > 1.0E-6;
        }
        return overlapsAll || Shapes.joinIsNotEmpty(first.move(new Vec3i(this.cubeX, this.cubeY, this.cubeZ)), second, op);
    }

    @Redirect(method = "tick", at = @At(value = "INVOKE",
            target = "Lnet/minecraft/world/level/Level;getEntities(Lnet/minecraft/world/entity/Entity;Lnet/minecraft/world/phys/AABB;Ljava/util/function/Predicate;)Ljava/util/List;"))
    private List<Entity> findPushed(Level level, Entity except, AABB box, Predicate<? super Entity> selector) {
        return level.isClientSide() && Initializer.CONFIG.fastBoatTicks
                ? level.getPushableEntities(except, box)
                : level.getEntities(except, box, selector);
    }
}
