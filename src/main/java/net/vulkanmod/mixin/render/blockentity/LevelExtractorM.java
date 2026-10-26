package net.vulkanmod.mixin.render.blockentity;

import net.minecraft.client.Camera;
import net.minecraft.client.renderer.blockentity.BlockEntityRenderDispatcher;
import net.minecraft.client.renderer.blockentity.state.BlockEntityRenderState;
import net.minecraft.client.renderer.culling.Frustum;
import net.minecraft.client.renderer.extract.LevelExtractor;
import net.minecraft.client.renderer.feature.ModelFeatureRenderer;
import net.minecraft.client.renderer.state.level.LevelRenderState;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.vulkanmod.render.blockentity.BlockEntityCulling;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.Redirect;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/**
 * Vanilla only culls block entities per section (frustum and occlusion), so every block entity of a partly visible
 * section is extracted and rendered. This additionally frustum culls each block entity, see {@link BlockEntityCulling}.
 */
@Mixin(LevelExtractor.class)
public class LevelExtractorM {
    @Unique private Frustum cullFrustum;

    @Inject(method = "extractVisibleBlockEntities", at = @At("HEAD"))
    private void captureFrustum(Camera camera, float deltaPartialTick, LevelRenderState levelRenderState, CallbackInfo ci) {
        this.cullFrustum = camera.getCullFrustum();
    }

    @Redirect(method = "extractVisibleBlockEntities", at = @At(value = "INVOKE", ordinal = 0,
            target = "Lnet/minecraft/client/renderer/blockentity/BlockEntityRenderDispatcher;tryExtractRenderState(Lnet/minecraft/world/level/block/entity/BlockEntity;FLnet/minecraft/client/renderer/feature/ModelFeatureRenderer$CrumblingOverlay;Z)Lnet/minecraft/client/renderer/blockentity/state/BlockEntityRenderState;"))
    private BlockEntityRenderState cullOutsideFrustum(BlockEntityRenderDispatcher dispatcher, BlockEntity blockEntity,
                                                      float partialTick, ModelFeatureRenderer.CrumblingOverlay breakProgress,
                                                      boolean isGloballyRendered) {
        if (BlockEntityCulling.isOutsideFrustum(this.cullFrustum, blockEntity)) {
            return null;
        }

        return dispatcher.tryExtractRenderState(blockEntity, partialTick, breakProgress, isGloballyRendered);
    }
}
