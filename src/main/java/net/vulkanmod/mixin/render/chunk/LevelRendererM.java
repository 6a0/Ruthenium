package net.vulkanmod.mixin.render.chunk;

import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.llamalad7.mixinextras.injector.wrapoperation.WrapOperation;
import com.llamalad7.mixinextras.sugar.Local;
import com.mojang.renderpearl.api.textures.GpuTextureView;
import it.unimi.dsi.fastutil.objects.ObjectArrayList;
import net.minecraft.client.renderer.DynamicGpuData;
import net.minecraft.client.renderer.LevelRenderer;
import net.minecraft.client.renderer.ViewArea;
import net.minecraft.client.renderer.chunk.ChunkSectionLayer;
import net.minecraft.client.renderer.chunk.ChunkSectionsToRender;
import net.minecraft.client.renderer.chunk.SectionMesh;
import net.minecraft.client.renderer.chunk.SectionRenderDispatcher;
import net.minecraft.client.renderer.state.OptionsRenderState;
import net.minecraft.client.renderer.state.level.LevelRenderState;
import net.minecraft.client.renderer.texture.TextureAtlas;
import net.minecraft.client.renderer.texture.TextureManager;
import net.minecraft.core.BlockPos;
import net.minecraft.world.phys.Vec3;
import net.vulkanmod.Initializer;
import net.vulkanmod.interfaces.FaceGroupsHolder;
import net.vulkanmod.interfaces.SectionDispatcherReset;
import net.vulkanmod.render.chunk.CompressedTerrain;
import net.vulkanmod.render.chunk.SectionFaceGroups;
import net.vulkanmod.render.chunk.TerrainDrawList;
import org.joml.Matrix4fc;
import org.jspecify.annotations.Nullable;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Overwrite;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import java.util.List;

@Mixin(LevelRenderer.class)
public abstract class LevelRendererM implements SectionDispatcherReset {
    @Shadow @Final private LevelRenderState levelRenderState;
    @Shadow @Final private OptionsRenderState optionsRenderState;
    @Shadow @Final private TextureManager textureManager;
    @Shadow @Final private ObjectArrayList<SectionRenderDispatcher.RenderSection> visibleSections;
    @Shadow private @Nullable SectionRenderDispatcher sectionRenderDispatcher;
    @Shadow private @Nullable ViewArea viewArea;

    @Shadow public abstract void clearVisibleSections();

    @Override
    public void vulkanmod$recreateSectionDispatcher() {
        // Like resetLevelRenderData, without resetting the section occlusion graph's loaded chunks and empty
        // sections, which are only filled as chunks load
        if (this.viewArea != null) {
            this.viewArea.releaseAllBuffers();
            this.viewArea = null;
        }
        if (this.sectionRenderDispatcher != null) {
            this.sectionRenderDispatcher.dispose();
            this.sectionRenderDispatcher = null;
        }
        this.clearVisibleSections();
    }

    /** Chooses the terrain vertex format of the section dispatcher this is about to create */
    @Inject(method = "invalidateCompiledGeometry", at = @At("HEAD"))
    private void chooseVertexFormat(CallbackInfo ci) {
        if (this.sectionRenderDispatcher == null) {
            CompressedTerrain.onDispatcherCreated();
        }
    }

    /**
     * @author VulkanMod
     * @reason Builds the same multi-draw-indirect commands without the per-frame hash lookups and records
     * ({@link TerrainDrawList}), and with the face groups
     */
    @Overwrite
    public ChunkSectionsToRender prepareChunkRendersIndirect(final Matrix4fc modelViewMatrix, final boolean respectTranslucentOrder) {
        GpuTextureView blockAtlas = this.textureManager.getTexture(TextureAtlas.LOCATION_BLOCKS).getTextureView();
        return TerrainDrawList.prepare(this.sectionRenderDispatcher, this.visibleSections, this.levelRenderState.cameraRenderState.pos,
                this.optionsRenderState.chunkSectionFadeInTime, modelViewMatrix, blockAtlas.getWidth(0), blockAtlas.getHeight(0),
                respectTranslucentOrder);
    }

    /**
     * Replaces the draw of a section layer whose quads are grouped by facing with draws of the groups that can face
     * the camera, on the path without multi-draw-indirect.
     */
    @SuppressWarnings({"rawtypes", "unchecked"})
    @WrapOperation(method = "extractSectionDrawGroups", at = @At(value = "INVOKE", target = "Ljava/util/List;add(Ljava/lang/Object;)Z"))
    private boolean addFacingGroupDraws(List list, Object element, Operation<Boolean> original,
                                        @Local SectionRenderDispatcher.RenderSection section,
                                        @Local ChunkSectionLayer layer, @Local SectionMesh sectionMesh) {
        if (element instanceof DynamicGpuData.IndexedDraw draw && Initializer.CONFIG.terrainFaceCulling
                && sectionMesh instanceof FaceGroupsHolder holder) {
            SectionFaceGroups groups = holder.vulkanmod$getFaceGroups(layer);
            SectionMesh.SectionDraw sectionDraw = sectionMesh.getSectionDraw(layer);
            if (groups != null && sectionDraw != null && !sectionDraw.hasCustomIndexBuffer()
                    && draw.indexCount() == groups.quadCount() * 6) {
                Vec3 eye = this.levelRenderState.cameraRenderState.pos;
                BlockPos origin = section.getRenderOrigin();
                groups.addVisibleDraws(list, draw, (float) (eye.x - origin.getX()), (float) (eye.y - origin.getY()),
                        (float) (eye.z - origin.getZ()));
                return true;
            }
        }
        return original.call(list, element);
    }
}
