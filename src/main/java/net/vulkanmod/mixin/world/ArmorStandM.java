package net.vulkanmod.mixin.world;

import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.decoration.ArmorStand;
import net.minecraft.world.level.Level;
import net.minecraft.world.phys.AABB;
import net.vulkanmod.world.EntityTypeTests;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Redirect;

import java.util.List;
import java.util.function.Predicate;

/**
 * Armor stands push only rideable minecarts, but query every entity around them every tick to find them; packed
 * armor stands then scan each other. The query typed to minecarts returns the same entities in the same order:
 * an entity section's class buckets keep insertion order, the armor stand itself isn't a minecart, and dragon parts
 * (which the untyped query adds) aren't either.
 */
@Mixin(ArmorStand.class)
public class ArmorStandM {
    @SuppressWarnings({"unchecked", "rawtypes"})
    @Redirect(method = "pushEntities", at = @At(value = "INVOKE",
            target = "Lnet/minecraft/world/level/Level;getEntities(Lnet/minecraft/world/entity/Entity;Lnet/minecraft/world/phys/AABB;Ljava/util/function/Predicate;)Ljava/util/List;"))
    private List<Entity> findMinecarts(Level level, Entity except, AABB box, Predicate<? super Entity> selector) {
        return (List) level.getEntities(EntityTypeTests.MINECARTS, box, (Predicate) selector);
    }
}
