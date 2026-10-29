package net.ruthenium.gametest.probemixin;

import com.llamalad7.mixinextras.injector.wrapmethod.WrapMethod;
import com.llamalad7.mixinextras.injector.wrapoperation.WrapOperation;
import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.mojang.blaze3d.systems.RenderSystem;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.ExperienceOrb;
import net.minecraft.world.entity.MoverType;
import net.minecraft.world.entity.item.ItemEntity;
import net.minecraft.world.level.Level;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.shapes.VoxelShape;
import net.minecraft.world.phys.Vec3;
import net.ruthenium.gametest.EntityCostProbe;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;

import java.util.List;

/** Parts of the client's entity tick (render thread only; the integrated server ticks the same classes) */
@Mixin(Entity.class)
public class EntityTickPartsProbeMixin {
    @WrapMethod(method = "baseTick")
    private void timeBaseTick(Operation<Void> original) {
        long start = System.nanoTime();
        original.call();
        if (RenderSystem.isOnRenderThread()) {
            EntityCostProbe.TICK_PARTS.add("Entity.baseTick", System.nanoTime() - start, 0);
        }
    }

    @WrapMethod(method = "move")
    private void timeMove(MoverType type, Vec3 delta, Operation<Void> original) {
        long start = System.nanoTime();
        original.call(type, delta);
        if (RenderSystem.isOnRenderThread()) {
            EntityCostProbe.TICK_PARTS.add("Entity.move", System.nanoTime() - start, 0);
        }
    }

    /** Items and orbs apart: resting ones collide with the floor every move */
    @WrapMethod(method = "collide")
    private Vec3 timeCollide(Vec3 movement, Operation<Vec3> original) {
        long start = System.nanoTime();
        Vec3 result = original.call(movement);
        if (RenderSystem.isOnRenderThread()) {
            EntityCostProbe.TICK_PARTS.add(COLLIDE[kind((Entity) (Object) this)], System.nanoTime() - start, 0);
        }
        return result;
    }

    @WrapOperation(method = "collide", at = @At(value = "INVOKE",
            target = "Lnet/minecraft/world/entity/Entity;collideBoundingBox(Lnet/minecraft/world/entity/Entity;Lnet/minecraft/world/phys/Vec3;Lnet/minecraft/world/phys/AABB;Lnet/minecraft/world/level/Level;Ljava/util/List;)Lnet/minecraft/world/phys/Vec3;"))
    private Vec3 timeCollideBlocks(Entity source, Vec3 movement, AABB box, Level level, List<VoxelShape> entityColliders,
                                   Operation<Vec3> original) {
        long start = System.nanoTime();
        Vec3 result = original.call(source, movement, box, level, entityColliders);
        if (RenderSystem.isOnRenderThread()) {
            EntityCostProbe.TICK_PARTS.add(COLLIDE_BLOCKS[kind(source)], System.nanoTime() - start, 0);
        }
        return result;
    }

    private static final String[] COLLIDE = {"Entity.collide (items)", "Entity.collide (orbs)", "Entity.collide (other)"};
    private static final String[] COLLIDE_BLOCKS = {"Entity.collideBoundingBox (items)", "Entity.collideBoundingBox (orbs)",
            "Entity.collideBoundingBox (other)"};

    private static int kind(Entity entity) {
        return entity instanceof ItemEntity ? 0 : entity instanceof ExperienceOrb ? 1 : 2;
    }

    @WrapMethod(method = "recordMovement")
    private void timeRecordMovement(MoverType type, Vec3 delta, Operation<Void> original) {
        long start = System.nanoTime();
        original.call(type, delta);
        if (RenderSystem.isOnRenderThread()) {
            EntityCostProbe.TICK_PARTS.add("Entity.recordMovement", System.nanoTime() - start, 0);
        }
    }

    @WrapMethod(method = "updateFluidInteraction")
    private boolean timeFluids(Operation<Boolean> original) {
        long start = System.nanoTime();
        boolean result = original.call();
        if (RenderSystem.isOnRenderThread()) {
            EntityCostProbe.TICK_PARTS.add("Entity.updateFluidInteraction", System.nanoTime() - start, 0);
        }
        return result;
    }

    @WrapMethod(method = "applyEffectsFromBlocks()V")
    private void timeBlockEffects(Operation<Void> original) {
        long start = System.nanoTime();
        original.call();
        if (RenderSystem.isOnRenderThread()) {
            EntityCostProbe.TICK_PARTS.add("Entity.applyEffectsFromBlocks", System.nanoTime() - start, 0);
        }
    }

    @WrapMethod(method = "commonTick")
    private void timeCommonTick(Operation<Void> original) {
        long start = System.nanoTime();
        original.call();
        if (RenderSystem.isOnRenderThread()) {
            EntityCostProbe.TICK_PARTS.add("Entity.commonTick", System.nanoTime() - start, 0);
        }
    }
}
