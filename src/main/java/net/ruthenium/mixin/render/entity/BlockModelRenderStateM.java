package net.ruthenium.mixin.render.entity;

import com.mojang.blaze3d.vertex.PoseStack;
import it.unimi.dsi.fastutil.ints.IntList;
import it.unimi.dsi.fastutil.objects.ObjectArrayList;
import net.fabricmc.fabric.api.client.renderer.v1.mesh.Mesh;
import net.minecraft.client.renderer.SubmitNodeCollector;
import net.minecraft.client.renderer.block.BlockModelRenderState;
import net.minecraft.client.renderer.block.dispatch.BlockStateModelPart;
import net.minecraft.client.renderer.rendertype.RenderType;
import net.minecraft.util.LightCoordsUtil;
import net.ruthenium.interfaces.CachedMeshRenderState;
import net.ruthenium.render.entity.BlockModelMeshCache;
import org.joml.Matrix4fc;
import org.jspecify.annotations.Nullable;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import java.util.Collections;
import java.util.List;

/**
 * Submits the cached mesh of a block model ({@link BlockModelMeshCache}) itself: the Fabric Renderer API copies the
 * model's quads into the render state's mesh while extracting and that mesh into a new immutable one while
 * submitting, for every entity and frame. The cached mesh is immutable and has the same quads. Applied after the
 * Fabric Renderer API's mixin (priority), whose submit does nothing while the render state's own mesh is empty.
 */
@Mixin(value = BlockModelRenderState.class, priority = 1100)
public class BlockModelRenderStateM implements CachedMeshRenderState {
    @Shadow private @Nullable List<BlockStateModelPart> modelParts;
    @Shadow private @Nullable Matrix4fc transformation;
    @Shadow private @Nullable IntList tintLayers;
    @Shadow public int blockLightCoords;

    @Unique private @Nullable Mesh cachedMesh;

    @Override
    public void ruthenium$setCachedMesh(@Nullable Mesh mesh) {
        this.cachedMesh = mesh;
    }

    @Inject(method = {"clear", "setupModel"}, at = @At("HEAD"))
    private void clearCachedMesh(CallbackInfo ci) {
        this.cachedMesh = null;
    }

    @Inject(method = "submitModel", at = @At("HEAD"), cancellable = true)
    private void submitCachedMesh(RenderType renderType, PoseStack poseStack, SubmitNodeCollector submitNodeCollector, int externalLightCoords,
                                  int overlayCoords, int outlineColor, CallbackInfo ci) {
        Mesh mesh = this.cachedMesh;
        if (mesh == null || mesh.size() == 0) {
            return;
        }

        // As the Fabric Renderer API's submit of the render state's mesh
        List<BlockStateModelPart> modelPartsCopy = this.modelParts != null && !this.modelParts.isEmpty()
                ? new ObjectArrayList<>(this.modelParts) : Collections.emptyList();
        int[] tints = this.tintLayers != null ? this.tintLayers.toArray(BlockModelRenderState.EMPTY_TINTS) : BlockModelRenderState.EMPTY_TINTS;
        int lightCoords = LightCoordsUtil.max(externalLightCoords, this.blockLightCoords);
        if (this.transformation != null) {
            poseStack.pushPose();
            poseStack.mulPose(this.transformation);
            submitNodeCollector.submitBlockModel(poseStack, _ -> renderType, renderType.hasBlending(), modelPartsCopy, mesh, tints, lightCoords,
                    overlayCoords, outlineColor);
            poseStack.popPose();
        } else {
            submitNodeCollector.submitBlockModel(poseStack, _ -> renderType, renderType.hasBlending(), modelPartsCopy, mesh, tints, lightCoords,
                    overlayCoords, outlineColor);
        }
        ci.cancel();
    }
}
