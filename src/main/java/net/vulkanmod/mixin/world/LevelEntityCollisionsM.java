package net.vulkanmod.mixin.world;

import com.google.common.collect.ImmutableList;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.EntitySelector;
import net.minecraft.world.entity.boss.enderdragon.EnderDragonPart;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.entity.LevelEntityGetter;
import net.minecraft.world.level.entity.LevelEntityGetterAdapter;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.shapes.Shapes;
import net.minecraft.world.phys.shapes.VoxelShape;
import net.vulkanmod.Initializer;
import net.vulkanmod.world.CollidableEntities;
import org.jspecify.annotations.Nullable;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.Unique;

import java.util.ArrayList;
import java.util.Collection;
import java.util.List;
import java.util.function.Predicate;

/**
 * Levels answer {@code getEntityCollisions} (the default method of {@code EntityGetter}, through
 * {@code CommonLevelAccessor}) themselves: the same code, but the entity search skips entity sections without
 * collidable entities ({@link CollidableEntities}) when the predicate can only accept those, i.e. for a null source
 * and for sources whose {@code canCollideWith} is {@link Entity}'s. Every moving entity's {@code collide} and every
 * {@code noCollision} (experience orbs twice per tick, interpolated movement, player pose checks) runs this query,
 * and in crowded places vanilla's search tests the bounding box of every entity in the surrounding sections.
 */
@Mixin(Level.class)
public abstract class LevelEntityCollisionsM {
    @Shadow protected abstract LevelEntityGetter<Entity> getEntities();
    @Shadow public abstract Collection<EnderDragonPart> dragonParts();
    @Shadow public abstract List<Entity> getEntities(@Nullable Entity except, AABB bb, Predicate<? super Entity> selector);

    public List<VoxelShape> getEntityCollisions(@Nullable Entity source, AABB testArea) {
        if (testArea.getSize() < 1.0E-7) {
            return List.of();
        }

        Predicate<Entity> canCollide = source == null ? EntitySelector.CAN_BE_COLLIDED_WITH : EntitySelector.NO_SPECTATORS.and(source::canCollideWith);
        List<Entity> collidingEntities;
        if (Initializer.CONFIG.fastEntityCollisions && (source == null || CollidableEntities.usesDefaultCanCollideWith(source))
                && this.getEntities() instanceof LevelEntityGetterAdapter<Entity> adapter) {
            collidingEntities = this.getCollidableEntities(adapter, source, testArea.inflate(1.0E-7), canCollide);
        } else {
            collidingEntities = this.getEntities(source, testArea.inflate(1.0E-7), canCollide);
        }

        if (collidingEntities.isEmpty()) {
            return List.of();
        }

        ImmutableList.Builder<VoxelShape> shapes = ImmutableList.builderWithExpectedSize(collidingEntities.size());
        for (Entity entity : collidingEntities) {
            shapes.add(Shapes.create(entity.getBoundingBox()));
        }
        return shapes.build();
    }

    /** {@code Level.getEntities(except, bb, selector)}, searching only sections with collidable entities */
    @Unique
    private List<Entity> getCollidableEntities(LevelEntityGetterAdapter<Entity> adapter, @Nullable Entity except, AABB bb,
                                               Predicate<Entity> selector) {
        List<Entity> output = new ArrayList<>();
        @SuppressWarnings("unchecked")
        var accessor = (LevelEntityGetterAdapterAccessor<Entity>) adapter;
        CollidableEntities.getEntities(accessor.vulkanmod$sectionStorage(), bb, entity -> {
            if (entity != except && selector.test(entity)) {
                output.add(entity);
            }
        });

        for (EnderDragonPart dragonPart : this.dragonParts()) {
            if (dragonPart != except && dragonPart.parentMob != except && selector.test(dragonPart) && bb.intersects(dragonPart.getBoundingBox())) {
                output.add(dragonPart);
            }
        }
        return output;
    }
}
