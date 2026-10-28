package net.vulkanmod.world;

import net.minecraft.world.entity.Entity;
import net.minecraft.world.level.entity.EntityAccess;
import net.minecraft.world.level.entity.EntitySectionStorage;
import net.minecraft.world.phys.AABB;
import net.minecraft.util.AbortableIterationConsumer;
import net.minecraft.util.Continuation;
import net.vulkanmod.interfaces.CollidableEntityCount;

import java.util.function.Consumer;

/**
 * Entity collision queries ({@code getEntityCollisions}: every moving entity's {@code collide}, every
 * {@code noCollision}) look for entities that {@code canBeCollidedWith} the source: boats, shulkers, happy ghasts
 * and modded classes that override that method. Every other entity's answer is {@link Entity}'s {@code false}. Each
 * entity section counts the entities of overriding classes, and the query skips sections without any: the entities
 * in them would all be rejected, so the result is the same list in the same order (an idea from Lithium's entity
 * class groups, implemented here independently).
 */
public final class CollidableEntities {
    private static final ClassValue<Boolean> CAN_BE_COLLIDED = new ClassValue<>() {
        @Override
        protected Boolean computeValue(Class<?> type) {
            return declaredBelowEntity(type, "canBeCollidedWith");
        }
    };
    private static final ClassValue<Boolean> CUSTOM_CAN_COLLIDE_WITH = new ClassValue<>() {
        @Override
        protected Boolean computeValue(Class<?> type) {
            return declaredBelowEntity(type, "canCollideWith");
        }
    };

    private CollidableEntities() {
    }

    /** Whether entities of this class may answer {@code canBeCollidedWith} with something else than Entity's false */
    public static boolean mayBeCollidable(Object entity) {
        return entity instanceof Entity && CAN_BE_COLLIDED.get(entity.getClass());
    }

    /**
     * Whether a source's {@code canCollideWith} can be decided by the other entity's {@code canBeCollidedWith}
     * alone, as {@link Entity}'s does (boats and minecarts also collide with pushable entities)
     */
    public static boolean usesDefaultCanCollideWith(Entity source) {
        return !CUSTOM_CAN_COLLIDE_WITH.get(source.getClass());
    }

    /** {@code EntitySectionStorage.getEntities(bb, consumer)} over the sections that hold collidable entities */
    public static <T extends EntityAccess> void getEntities(EntitySectionStorage<T> storage, AABB bb, Consumer<T> output) {
        AbortableIterationConsumer<T> consumer = AbortableIterationConsumer.forConsumer(output);
        storage.forEachAccessibleNonEmptySection(bb, section -> ((CollidableEntityCount) section).vulkanmod$collidableEntities() > 0
                ? section.getEntities(bb, consumer)
                : Continuation.CONTINUE);
    }

    private static boolean declaredBelowEntity(Class<?> type, String name) {
        try {
            return type.getMethod(name, Entity.class).getDeclaringClass() != Entity.class;
        } catch (ReflectiveOperationException | RuntimeException e) {
            return true;
        }
    }
}
