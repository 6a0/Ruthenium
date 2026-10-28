package net.ruthenium.mixin.render.entity;

import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.llamalad7.mixinextras.injector.wrapoperation.WrapOperation;
import net.minecraft.client.Camera;
import net.minecraft.client.entity.ClientMannequin;
import net.minecraft.client.player.AbstractClientPlayer;
import net.ruthenium.Initializer;
import net.ruthenium.interfaces.EntityRendererMap;
import net.ruthenium.render.entity.EntityRendererCache;
import net.ruthenium.render.entity.TextDisplayCulling;
import net.minecraft.world.entity.Display;
import java.util.Map;
import net.minecraft.client.DeltaTracker;
import net.minecraft.client.renderer.LevelRenderer;
import net.minecraft.client.renderer.culling.Frustum;
import net.minecraft.client.renderer.entity.EntityRenderDispatcher;
import net.minecraft.client.renderer.entity.state.EntityRenderState;
import net.minecraft.client.renderer.extract.LevelExtractor;
import net.minecraft.client.renderer.state.level.LevelRenderState;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.decoration.ArmorStand;
import net.ruthenium.render.entity.ReusedRenderStates;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Overwrite;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/**
 * Armor stands are extracted into reused render states ({@link ReusedRenderStates}); entity frustum culling through
 * {@link EntityRendererCache}.
 */
@Mixin(LevelExtractor.class)
public class LevelExtractorEntityM {
    @Shadow @Final private LevelRenderer levelRenderer;

    @Inject(method = "extractVisibleEntities", at = @At("HEAD"))
    private void beginEntities(Camera camera, Frustum frustum, DeltaTracker deltaTracker, LevelRenderState output, CallbackInfo ci) {
        ReusedRenderStates.beginFrame();
    }

    /**
     * @author Ruthenium
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

    /**
     * Frustum culling of entities with plain renderers without allocating boxes or looking the renderer up
     * ({@link EntityRendererCache#isVisible}); the rest, and undecided cases, take vanilla's path.
     */
    @WrapOperation(method = "isEntityVisible", at = @At(value = "INVOKE",
            target = "Lnet/minecraft/client/renderer/entity/EntityRenderDispatcher;shouldRender(Lnet/minecraft/world/entity/Entity;Lnet/minecraft/client/renderer/culling/Frustum;DDDF)Z"))
    private boolean ruthenium$fastCulling(EntityRenderDispatcher dispatcher, Entity entity, Frustum frustum, double camX, double camY, double camZ,
                                          float partialTicks, Operation<Boolean> original) {
        if (Initializer.CONFIG.fastEntityCulling && !(entity instanceof AbstractClientPlayer) && !(entity instanceof ClientMannequin)) {
            Map<?, ?> renderers = ((EntityRendererMap) dispatcher).ruthenium$renderers();
            EntityRendererCache.Entry entry = EntityRendererCache.entry(entity.getType(), renderers);
            if (entry == null) {
                dispatcher.getRenderer(entity);
                entry = EntityRendererCache.entry(entity.getType(), renderers);
            }
            if (entry != null && entry.cullKind() != EntityRendererCache.CULL_VANILLA) {
                int visible = EntityRendererCache.isVisible(entry.cullKind(), entity, frustum, camX, camY, camZ, partialTicks);
                if (visible != EntityRendererCache.UNDECIDED) {
                    return visible == EntityRendererCache.VISIBLE;
                }
            }
        }
        boolean visible = original.call(dispatcher, entity, frustum, camX, camY, camZ, partialTicks);
        if (visible && entity instanceof Display.TextDisplay display
                && TextDisplayCulling.isHidden(display, frustum, camX, camY, camZ, partialTicks)) {
            return false;
        }
        return visible;
    }

    @Inject(method = {"setLevel", "allChanged"}, at = @At("HEAD"))
    private void clearStates(CallbackInfo ci) {
        ReusedRenderStates.clear();
    }
}
