package net.ruthenium.mixin.render.entity;

import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.llamalad7.mixinextras.injector.wrapoperation.WrapOperation;
import net.minecraft.client.renderer.entity.EntityRenderer;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Position;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.phys.Vec3;
import net.ruthenium.Initializer;
import net.ruthenium.render.entity.EntityLightProbe;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;

/** The light probe position of {@code getPackedLightCoords} without a new vector and block position ({@link EntityLightProbe}) */
@Mixin(EntityRenderer.class)
public class EntityRendererProbeM {
    @WrapOperation(method = "getPackedLightCoords", at = @At(value = "INVOKE",
            target = "Lnet/minecraft/world/entity/Entity;getLightProbePosition(F)Lnet/minecraft/world/phys/Vec3;"))
    private Vec3 ruthenium$probe(Entity entity, float partialTick, Operation<Vec3> original) {
        if (!Initializer.CONFIG.noAllocLightProbe) {
            return original.call(entity, partialTick);
        }
        return EntityLightProbe.probe(entity, partialTick);
    }

    @WrapOperation(method = "getPackedLightCoords", at = @At(value = "INVOKE",
            target = "Lnet/minecraft/core/BlockPos;containing(Lnet/minecraft/core/Position;)Lnet/minecraft/core/BlockPos;"))
    private BlockPos ruthenium$block(Position position, Operation<BlockPos> original) {
        if (EntityLightProbe.isMarker(position)) {
            return EntityLightProbe.block();
        }
        return Initializer.CONFIG.noAllocLightProbe ? BlockPos.containing(position) : original.call(position);
    }
}
