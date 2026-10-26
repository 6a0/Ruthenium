package net.vulkanmod.mixin.render.blockentity;

import it.unimi.dsi.fastutil.longs.Long2ObjectMap;
import net.minecraft.client.Camera;
import net.minecraft.client.renderer.blockentity.BlockEntityRenderDispatcher;
import net.minecraft.client.renderer.blockentity.state.BlockEntityRenderState;
import net.minecraft.client.renderer.chunk.SectionMesh;
import net.minecraft.client.renderer.chunk.SectionRenderDispatcher;
import net.minecraft.client.renderer.culling.Frustum;
import net.minecraft.client.renderer.extract.LevelExtractor;
import net.minecraft.client.renderer.feature.ModelFeatureRenderer;
import net.minecraft.client.renderer.state.level.LevelRenderState;
import net.minecraft.core.BlockPos;
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
 * <p>
 * The test happens where vanilla reads the block entity's position, so culled block entities also skip the
 * destruction progress lookup. Each section is classified once, and only block entities of sections that cross the
 * frustum are tested individually.
 */
@Mixin(LevelExtractor.class)
public class LevelExtractorM {
    @Unique private Frustum cullFrustum;
    @Unique private SectionRenderDispatcher.RenderSection cullSection;
    @Unique private int sectionCull;
    @Unique private boolean culled;

    @Inject(method = "extractVisibleBlockEntities", at = @At("HEAD"))
    private void captureFrustum(Camera camera, float deltaPartialTick, LevelRenderState levelRenderState, CallbackInfo ci) {
        this.cullFrustum = camera.getCullFrustum();
    }

    @Inject(method = "extractVisibleBlockEntities", at = @At("RETURN"))
    private void releaseSection(Camera camera, float deltaPartialTick, LevelRenderState levelRenderState, CallbackInfo ci) {
        this.cullSection = null;
        this.culled = false;
    }

    @Redirect(method = "extractVisibleBlockEntities", at = @At(value = "INVOKE", ordinal = 0,
            target = "Lnet/minecraft/client/renderer/chunk/SectionRenderDispatcher$RenderSection;getSectionMesh()Lnet/minecraft/client/renderer/chunk/SectionMesh;"))
    private SectionMesh enterSection(SectionRenderDispatcher.RenderSection section) {
        // Classified lazily on the first cullable block entity, most sections have none
        this.cullSection = section;
        this.sectionCull = BlockEntityCulling.SECTION_UNKNOWN;
        return section.getSectionMesh();
    }

    @Redirect(method = "extractVisibleBlockEntities", at = @At(value = "INVOKE", ordinal = 0,
            target = "Lnet/minecraft/world/level/block/entity/BlockEntity;getBlockPos()Lnet/minecraft/core/BlockPos;"))
    private BlockPos testBlockEntity(BlockEntity blockEntity) {
        this.culled = false;

        if (this.cullFrustum != null && BlockEntityCulling.isCullable(blockEntity.getType())) {
            if (this.sectionCull == BlockEntityCulling.SECTION_UNKNOWN) {
                this.sectionCull = BlockEntityCulling.classifySection(this.cullFrustum, this.cullSection.getRenderOrigin());
            }

            this.culled = switch (this.sectionCull) {
                case BlockEntityCulling.SECTION_INSIDE -> false;
                case BlockEntityCulling.SECTION_OUTSIDE -> true;
                default -> BlockEntityCulling.isOutsideFrustum(this.cullFrustum, blockEntity.getBlockPos());
            };
        }

        return blockEntity.getBlockPos();
    }

    @Redirect(method = "extractVisibleBlockEntities", at = @At(value = "INVOKE", ordinal = 0,
            target = "Lit/unimi/dsi/fastutil/longs/Long2ObjectMap;get(J)Ljava/lang/Object;"))
    private Object getDestructionProgress(Long2ObjectMap<?> destructionProgress, long pos) {
        if (this.culled || destructionProgress.isEmpty()) {
            return null;
        }

        return destructionProgress.get(pos);
    }

    @Redirect(method = "extractVisibleBlockEntities", at = @At(value = "INVOKE", ordinal = 0,
            target = "Lnet/minecraft/client/renderer/blockentity/BlockEntityRenderDispatcher;tryExtractRenderState(Lnet/minecraft/world/level/block/entity/BlockEntity;FLnet/minecraft/client/renderer/feature/ModelFeatureRenderer$CrumblingOverlay;Z)Lnet/minecraft/client/renderer/blockentity/state/BlockEntityRenderState;"))
    private BlockEntityRenderState cullOutsideFrustum(BlockEntityRenderDispatcher dispatcher, BlockEntity blockEntity,
                                                      float partialTick, ModelFeatureRenderer.CrumblingOverlay breakProgress,
                                                      boolean isGloballyRendered) {
        if (this.culled) {
            return null;
        }

        return dispatcher.tryExtractRenderState(blockEntity, partialTick, breakProgress, isGloballyRendered);
    }
}
