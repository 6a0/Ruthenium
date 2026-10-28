package net.ruthenium.mixin.render.entity;

import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.llamalad7.mixinextras.injector.wrapoperation.WrapOperation;
import com.llamalad7.mixinextras.sugar.Local;
import com.mojang.blaze3d.systems.RenderSystem;
import net.minecraft.client.renderer.entity.EntityRenderer;
import net.minecraft.core.BlockPos;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.LightLayer;
import net.ruthenium.Initializer;
import net.ruthenium.interfaces.CachedEntityLight;
import net.ruthenium.render.entity.EntityLight;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;

/**
 * The block and sky light that {@code EntityRenderer} reads at an entity's light probe position every frame come from
 * the values kept on the entity ({@link EntityLight}).
 */
@Mixin(EntityRenderer.class)
public class EntityRendererLightM {
    @WrapOperation(method = {"getSkyLightLevel", "getBlockLightLevel"}, at = @At(value = "INVOKE",
            target = "Lnet/minecraft/world/level/Level;getBrightness(Lnet/minecraft/world/level/LightLayer;Lnet/minecraft/core/BlockPos;)I"))
    private int ruthenium$keptBrightness(Level level, LightLayer layer, BlockPos pos, Operation<Integer> original, @Local(argsOnly = true) Entity entity) {
        if (!Initializer.CONFIG.entityLightCache || !RenderSystem.isOnRenderThread()) {
            return original.call(level, layer, pos);
        }

        CachedEntityLight cache = (CachedEntityLight) entity;
        int light = cache.ruthenium$light(level, layer, pos.asLong(), EntityLight.generation());
        if (light < 0) {
            light = original.call(level, layer, pos);
            cache.ruthenium$setLight(layer, light);
        }
        return light;
    }
}
