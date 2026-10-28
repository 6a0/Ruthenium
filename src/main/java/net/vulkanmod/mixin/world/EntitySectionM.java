package net.vulkanmod.mixin.world;

import net.minecraft.world.level.entity.EntityAccess;
import net.minecraft.world.level.entity.EntitySection;
import net.vulkanmod.interfaces.CollidableEntityCount;
import net.vulkanmod.world.CollidableEntities;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

/** Counts the section's entities that may be collided with, for {@link CollidableEntities} (always kept, any switch) */
@Mixin(EntitySection.class)
public class EntitySectionM<T extends EntityAccess> implements CollidableEntityCount {
    @Unique
    private int collidableEntities;

    @Inject(method = "add", at = @At("TAIL"))
    private void countAdded(T entity, CallbackInfo ci) {
        if (CollidableEntities.mayBeCollidable(entity)) {
            this.collidableEntities++;
        }
    }

    @Inject(method = "remove", at = @At("RETURN"))
    private void countRemoved(T entity, CallbackInfoReturnable<Boolean> cir) {
        if (cir.getReturnValueZ() && CollidableEntities.mayBeCollidable(entity)) {
            this.collidableEntities--;
        }
    }

    @Override
    public int vulkanmod$collidableEntities() {
        return this.collidableEntities;
    }
}
