package net.vulkanmod.mixin.render.chunk;

import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.llamalad7.mixinextras.injector.wrapoperation.WrapOperation;
import com.llamalad7.mixinextras.sugar.Local;
import net.minecraft.client.renderer.DynamicGpuData;
import net.minecraft.client.renderer.LevelRenderer;
import net.minecraft.client.renderer.chunk.ChunkSectionLayer;
import net.minecraft.client.renderer.chunk.SectionMesh;
import net.minecraft.client.renderer.chunk.SectionRenderDispatcher;
import net.minecraft.client.renderer.state.level.LevelRenderState;
import net.minecraft.core.BlockPos;
import net.minecraft.world.phys.Vec3;
import net.vulkanmod.Initializer;
import net.vulkanmod.interfaces.FaceGroupsHolder;
import net.vulkanmod.render.chunk.SectionFaceGroups;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;

import java.util.List;

@Mixin(LevelRenderer.class)
public class LevelRendererM {
    @Shadow @Final private LevelRenderState levelRenderState;

    /**
     * Replaces the draw of a section layer whose quads are grouped by facing with draws of the groups that can face
     * the camera (the multi-draw-indirect and the separate draw paths are both built from these draws).
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
