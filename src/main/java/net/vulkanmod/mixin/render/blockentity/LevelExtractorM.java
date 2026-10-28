package net.vulkanmod.mixin.render.blockentity;

import com.mojang.blaze3d.vertex.PoseStack;
import it.unimi.dsi.fastutil.longs.Long2ObjectMap;
import it.unimi.dsi.fastutil.objects.ObjectArrayList;
import net.minecraft.client.Camera;
import net.minecraft.client.Minecraft;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.renderer.LevelRenderer;
import net.minecraft.client.renderer.blockentity.BlockEntityRenderDispatcher;
import net.minecraft.client.renderer.blockentity.state.BlockEntityRenderState;
import net.minecraft.client.renderer.chunk.SectionMesh;
import net.minecraft.client.renderer.chunk.SectionRenderDispatcher;
import net.minecraft.client.renderer.culling.Frustum;
import net.minecraft.client.renderer.extract.LevelExtractor;
import net.minecraft.client.renderer.feature.ModelFeatureRenderer;
import net.minecraft.client.renderer.state.level.LevelRenderState;
import net.minecraft.core.BlockPos;
import net.minecraft.server.level.BlockDestructionProgress;
import net.minecraft.util.Util;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.entity.EnchantingTableBlockEntity;
import net.minecraft.world.phys.Vec3;
import net.vulkanmod.Initializer;
import net.vulkanmod.interfaces.BlockEntityPositions;
import net.vulkanmod.render.blockentity.BlockEntityCulling;
import net.vulkanmod.render.blockentity.InstancedModels;
import net.vulkanmod.render.blockentity.RetainedBlockEntities;
import org.jspecify.annotations.Nullable;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Overwrite;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import java.util.Iterator;
import java.util.List;
import java.util.SortedSet;

/**
 * Vanilla only culls block entities per section (frustum and occlusion), so every block entity of a partly visible
 * section is extracted and rendered. This additionally frustum culls each block entity, see {@link BlockEntityCulling}.
 * <p>
 * Each section is classified once; in sections that cross the frustum each 4³ cell is classified once, and only
 * block entities of cells that cross it are tested individually. Positions and cullability come from the compiled
 * section ({@link BlockEntityPositions}) by list index, so culled block entities are not even loaded from the list:
 * with ~14k block entities per frame the pointer chasing was the main cost. Block entities kept by
 * {@link RetainedBlockEntities} are skipped, and block entities drawn as {@link InstancedModels} are not submitted.
 * Otherwise this is vanilla's loop.
 */
@Mixin(LevelExtractor.class)
public class LevelExtractorM {
    @Shadow @Final private Minecraft minecraft;
    @Shadow @Final private LevelRenderer levelRenderer;
    @Shadow private @Nullable ClientLevel level;

    /** Cell classifications of the current partial section, valid where {@link #cellStamps} equals {@link #sectionStamp} */
    @Unique private final byte[] cellCull = new byte[64];
    @Unique private final int[] cellStamps = new int[64];
    @Unique private int sectionStamp;

    /**
     * @author VulkanMod
     * @reason Per block entity frustum culling without touching culled block entities
     */
    @Overwrite
    private void extractVisibleBlockEntities(final Camera camera, final float deltaPartialTick, final LevelRenderState levelRenderState) {
        Vec3 cameraPos = camera.position();
        double camX = cameraPos.x();
        double camY = cameraPos.y();
        double camZ = cameraPos.z();
        PoseStack poseStack = null;
        long chunkFadeDuration = Util.toMillis(this.minecraft.options.chunkSectionFadeInTime().get());
        // Null when per block entity culling is off
        Frustum frustum = Initializer.CONFIG.blockEntityCulling ? camera.getCullFrustum() : null;
        Long2ObjectMap<SortedSet<BlockDestructionProgress>> destructionProgress = this.level.destructionProgress();
        BlockEntityRenderDispatcher dispatcher = this.levelRenderer.blockEntityRenderDispatcher();
        List<BlockEntityRenderState> output = levelRenderState.blockEntityRenderStates;
        ObjectArrayList<SectionRenderDispatcher.RenderSection> sections = this.levelRenderer.visibleSections();
        RetainedBlockEntities.beginFrame(cameraPos);
        InstancedModels.beginFrame();

        for (int s = 0, sectionCount = sections.size(); s < sectionCount; s++) {
            SectionRenderDispatcher.RenderSection section = sections.get(s);
            SectionMesh mesh = section.getSectionMesh();
            List<BlockEntity> blockEntities = mesh.getRenderableBlockEntities();
            if (blockEntities.isEmpty() || section.getVisibility(Util.getMillis(), chunkFadeDuration) < 0.3F) {
                continue;
            }

            // Block entities whose geometry is kept on the GPU
            boolean[] retained = RetainedBlockEntities.section(section.getRenderOrigin(), mesh, blockEntities, this.level, dispatcher,
                    deltaPartialTick, cameraPos, levelRenderState.cameraRenderState);

            long[] positions = null;
            boolean[] cullable = null;
            if (frustum != null && mesh instanceof BlockEntityPositions packed) {
                positions = packed.vulkanmod$getPositions();
                cullable = packed.vulkanmod$getCullable();
            }

            // Classified lazily on the first cullable block entity
            int sectionCull = BlockEntityCulling.SECTION_UNKNOWN;
            this.sectionStamp++;

            for (int i = 0, size = blockEntities.size(); i < size; i++) {
                if (retained != null && retained[i]) {
                    continue;
                }

                BlockEntity blockEntity;
                if (positions != null && i < positions.length) {
                    if (cullable[i]) {
                        if (sectionCull == BlockEntityCulling.SECTION_UNKNOWN) {
                            sectionCull = BlockEntityCulling.classifySection(frustum, section.getRenderOrigin());
                        }
                        if (sectionCull == BlockEntityCulling.SECTION_OUTSIDE
                                || sectionCull == BlockEntityCulling.SECTION_PARTIAL && this.isCulledInPartialSection(frustum, positions[i])) {
                            continue;
                        }
                    }
                    blockEntity = blockEntities.get(i);
                } else {
                    // Not a compiled section mesh
                    blockEntity = blockEntities.get(i);
                    if (frustum != null && BlockEntityCulling.isCullable(blockEntity.getType())) {
                        BlockPos pos = blockEntity.getBlockPos();
                        if (BlockEntityCulling.isOutsideFrustum(frustum, pos.getX(), pos.getY(), pos.getZ())) {
                            continue;
                        }
                    }
                }

                BlockPos blockPos = blockEntity.getBlockPos();
                SortedSet<BlockDestructionProgress> progresses = destructionProgress.isEmpty() ? null : destructionProgress.get(blockPos.asLong());
                ModelFeatureRenderer.CrumblingOverlay breakProgress;
                if (progresses != null && !progresses.isEmpty()) {
                    if (poseStack == null) {
                        poseStack = new PoseStack();
                    }
                    poseStack.pushPose();
                    poseStack.translate(blockPos.getX() - camX, blockPos.getY() - camY, blockPos.getZ() - camZ);
                    breakProgress = new ModelFeatureRenderer.CrumblingOverlay(progresses.last().getProgress(), poseStack.last());
                    poseStack.popPose();
                } else {
                    breakProgress = null;
                    if (blockEntity instanceof EnchantingTableBlockEntity table
                            && InstancedModels.tryAddClosedBook(dispatcher, table, deltaPartialTick, camX, camY, camZ)) {
                        continue;
                    }
                }

                BlockEntityRenderState state = dispatcher.tryExtractRenderState(blockEntity, deltaPartialTick, breakProgress, false);
                if (state != null && !InstancedModels.tryAdd(dispatcher, state, camX, camY, camZ, levelRenderState.cameraRenderState)) {
                    output.add(state);
                }
            }
        }

        Iterator<BlockEntity> globalIterator = this.level.getGloballyRenderedBlockEntities().iterator();
        while (globalIterator.hasNext()) {
            BlockEntity blockEntity = globalIterator.next();
            if (blockEntity.isRemoved()) {
                globalIterator.remove();
            } else {
                BlockEntityRenderState state = dispatcher.tryExtractRenderState(blockEntity, deltaPartialTick, null, true);
                if (state != null) {
                    output.add(state);
                }
            }
        }
    }

    @Inject(method = "setSectionDirty(IIIZ)V", at = @At("HEAD"))
    private void markRetainedDirty(int sectionX, int sectionY, int sectionZ, boolean playerChanged, CallbackInfo ci) {
        RetainedBlockEntities.markBlocksChanged(sectionX, sectionY, sectionZ);
        InstancedModels.invalidateLight();
    }

    @Inject(method = {"setLevel", "allChanged"}, at = @At("HEAD"))
    private void clearRetained(CallbackInfo ci) {
        RetainedBlockEntities.clear();
        InstancedModels.clear();
    }

    @Unique
    private boolean isCulledInPartialSection(Frustum frustum, long packed) {
        int x = BlockPos.getX(packed), y = BlockPos.getY(packed), z = BlockPos.getZ(packed);
        int cell = BlockEntityCulling.cellIndex(x, y, z);
        int cellCull;
        if (this.cellStamps[cell] == this.sectionStamp) {
            cellCull = this.cellCull[cell];
        } else {
            cellCull = BlockEntityCulling.classifyCell(frustum, x, y, z);
            this.cellCull[cell] = (byte) cellCull;
            this.cellStamps[cell] = this.sectionStamp;
        }

        return switch (cellCull) {
            case BlockEntityCulling.SECTION_INSIDE -> false;
            case BlockEntityCulling.SECTION_OUTSIDE -> true;
            default -> BlockEntityCulling.isOutsideFrustum(frustum, x, y, z);
        };
    }
}
