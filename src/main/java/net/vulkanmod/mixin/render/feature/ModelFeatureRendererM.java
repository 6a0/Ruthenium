package net.vulkanmod.mixin.render.feature;

import com.mojang.blaze3d.vertex.PoseStack;
import com.mojang.blaze3d.vertex.SheetedDecalTextureGenerator;
import com.mojang.blaze3d.vertex.VertexConsumer;
import net.minecraft.client.model.Model;
import net.minecraft.client.renderer.feature.ModelFeatureRenderer;
import net.minecraft.client.renderer.feature.RenderTypeFeatureRenderer;
import net.vulkanmod.render.blockentity.InstancedModels;
import net.vulkanmod.render.entity.InstancedParts;
import net.vulkanmod.render.entity.ModelCulling;
import net.vulkanmod.render.entity.ModelGeometryCache;
import net.vulkanmod.render.entity.ModelPoseReplay;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Overwrite;
import org.spongepowered.asm.mixin.Shadow;

@Mixin(ModelFeatureRenderer.class)
public abstract class ModelFeatureRendererM extends RenderTypeFeatureRenderer<ModelFeatureRenderer.Submit<?>> {
    @Shadow @Final private PoseStack poseStack;

    /**
     * @author VulkanMod
     * @reason Instancing ({@link InstancedModels}), back face culling ({@link ModelCulling}), cached rest pose and
     * model space geometry ({@link ModelGeometryCache}), instanced parts of posed models ({@link InstancedParts}) and part
     * poses of layers ({@link ModelPoseReplay});
     * otherwise the vanilla body
     */
    @Overwrite
    private <S> void prepareModel(ModelFeatureRenderer.Submit<S> submit) {
        ModelCulling.beginModel(submit.renderType());
        try {
            if (InstancedModels.tryAdd(submit)) {
                return;
            }

            this.poseStack.last().set(submit.pose());
            VertexConsumer buffer = this.getVertexBuilder(submit.renderType());
            Model<? super S> model = submit.model();

            if (submit.sheetedDecalPose() == null && ModelGeometryCache.isCacheable(submit.state())
                    && ModelGeometryCache.write(model, submit.state(), this.poseStack.last(), buffer, submit.lightCoords(),
                            submit.overlayCoords(), submit.tintedColor(), submit.uvMapping(), ModelCulling.cullBackFaces())) {
                return;
            }

            Object modelSpaceKey;
            if (submit.sheetedDecalPose() == null && (modelSpaceKey = ModelGeometryCache.modelSpaceKey(submit.state())) != null
                    && ModelGeometryCache.writeModelSpace(model, submit.state(), modelSpaceKey, this.poseStack.last(), buffer,
                            submit.lightCoords(), submit.overlayCoords(), submit.tintedColor(), submit.uvMapping(),
                            ModelCulling.cullBackFaces(), ModelCulling.skipDuplicateQuads())) {
                return;
            }

            if (submit.sheetedDecalPose() != null) {
                buffer = new SheetedDecalTextureGenerator(buffer, submit.sheetedDecalPose(), 1.0F);
            } else if (submit.uvMapping() != null) {
                buffer = submit.uvMapping().wrap(buffer);
            }

            boolean instancedParts = submit.sheetedDecalPose() == null && InstancedParts.beginModel(submit.renderType(), submit.uvMapping());
            try {
                // A layer of a model posed earlier in this frame for the same state takes the recorded part poses
                ModelPoseReplay.Record replay = ModelPoseReplay.begin(model, submit.state(), this.poseStack.last());
                if (replay != null) {
                    replay.replay(buffer, submit.lightCoords(), submit.overlayCoords(), submit.tintedColor());
                } else {
                    model.setupAnim(submit.state());
                    model.renderToBuffer(this.poseStack, buffer, submit.lightCoords(), submit.overlayCoords(), submit.tintedColor());
                    ModelPoseReplay.end();
                }
            } finally {
                if (instancedParts) {
                    InstancedParts.endModel();
                }
            }
        } finally {
            ModelCulling.endModel();
        }
    }
}
