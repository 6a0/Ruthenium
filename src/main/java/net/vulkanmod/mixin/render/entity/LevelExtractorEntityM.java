package net.vulkanmod.mixin.render.entity;

import net.minecraft.client.Camera;
import net.minecraft.client.DeltaTracker;
import net.minecraft.client.renderer.LevelRenderer;
import net.minecraft.client.renderer.culling.Frustum;
import net.minecraft.client.renderer.entity.EntityRenderDispatcher;
import net.minecraft.client.renderer.entity.state.EntityRenderState;
import net.minecraft.client.renderer.extract.LevelExtractor;
import net.minecraft.client.renderer.state.level.LevelRenderState;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.decoration.ArmorStand;
import net.vulkanmod.render.entity.ReusedRenderStates;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Overwrite;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/**
 * Armor stands are extracted into reused render states ({@link ReusedRenderStates}).
 */
@Mixin(LevelExtractor.class)
public class LevelExtractorEntityM {
    @Shadow @Final private LevelRenderer levelRenderer;

    @Inject(method = "extractVisibleEntities", at = @At("HEAD"))
    private void beginEntities(Camera camera, Frustum frustum, DeltaTracker deltaTracker, LevelRenderState output, CallbackInfo ci) {
        ReusedRenderStates.beginFrame();
    }

    /**
     * @author VulkanMod
     * @reason Armor stands extracted into reused render states
     */
    @Overwrite
    private EntityRenderState extractEntity(final Entity entity, final float partialTickTime) {
        EntityRenderDispatcher dispatcher = this.levelRenderer.entityRenderDispatcher();
        if (entity.getClass() == ArmorStand.class) {
            try {
                EntityRenderState state = ReusedRenderStates.extract(dispatcher, (ArmorStand) entity, partialTickTime);
                if (state != null) {
                    return state;
                }
            } catch (Throwable t) {
                // Vanilla's path below reports it
            }
        }
        return dispatcher.extractEntity(entity, partialTickTime);
    }

    @Inject(method = {"setLevel", "allChanged"}, at = @At("HEAD"))
    private void clearStates(CallbackInfo ci) {
        ReusedRenderStates.clear();
    }
}
