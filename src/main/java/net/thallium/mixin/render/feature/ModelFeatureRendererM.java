package net.thallium.mixin.render.feature;

import com.mojang.blaze3d.vertex.PoseStack;
import com.mojang.blaze3d.vertex.SheetedDecalTextureGenerator;
import com.mojang.blaze3d.vertex.VertexConsumer;
import net.minecraft.client.model.Model;
import net.minecraft.client.renderer.feature.ModelFeatureRenderer;
import net.minecraft.client.renderer.feature.RenderTypeFeatureRenderer;
import net.thallium.render.blockentity.InstancedModels;
import net.thallium.render.entity.InstancedParts;
import net.thallium.render.entity.LazyVertexBuilder;
import net.thallium.render.entity.ModelCulling;
import net.thallium.render.entity.ModelGeometryCache;
import net.thallium.render.entity.ModelPoseReplay;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Overwrite;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.Unique;
import org.jspecify.annotations.Nullable;

@Mixin(ModelFeatureRenderer.class)
public abstract class ModelFeatureRendererM extends RenderTypeFeatureRenderer<ModelFeatureRenderer.Submit<?>> {
    @Shadow @Final private PoseStack poseStack;
    /** The vertex builder of submits whose parts become instances, reused for each */
    @Unique private @Nullable LazyVertexBuilder lazyBuilder;

    /**
     * @author Thallium
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
            Model<? super S> model = submit.model();
            boolean decal = submit.sheetedDecalPose() != null;
            VertexConsumer buffer = null;

            if (!decal && ModelGeometryCache.isCacheable(submit.state())) {
                buffer = this.getVertexBuilder(submit.renderType());
                if (ModelGeometryCache.write(model, submit.state(), this.poseStack.last(), buffer, submit.lightCoords(),
                        submit.overlayCoords(), submit.tintedColor(), submit.uvMapping(), ModelCulling.cullBackFaces())) {
                    return;
                }
            }

            Object modelSpaceKey;
            if (!decal && (modelSpaceKey = ModelGeometryCache.modelSpaceKey(submit.state())) != null) {
                if (buffer == null) {
                    buffer = this.getVertexBuilder(submit.renderType());
                }
                if (ModelGeometryCache.writeModelSpace(model, submit.state(), modelSpaceKey, this.poseStack.last(), buffer,
                        submit.lightCoords(), submit.overlayCoords(), submit.tintedColor(), submit.uvMapping(),
                        ModelCulling.cullBackFaces(), ModelCulling.skipDuplicateQuads())) {
                    return;
                }
            }

            boolean instancedParts = !decal && InstancedParts.beginModel(submit.renderType(), submit.uvMapping());
            if (instancedParts && buffer == null) {
                // The parts become instances: the group's vertex builder (a render type switch) is only fetched if
                // one of them has to be written after all
                LazyVertexBuilder lazy = this.lazyBuilder;
                if (lazy == null) {
                    lazy = this.lazyBuilder = new LazyVertexBuilder(this::getVertexBuilder);
                }
                buffer = lazy.reset(submit.renderType(), submit.uvMapping());
            } else {
                if (buffer == null) {
                    buffer = this.getVertexBuilder(submit.renderType());
                }
                if (decal) {
                    buffer = new SheetedDecalTextureGenerator(buffer, submit.sheetedDecalPose(), 1.0F);
                } else if (submit.uvMapping() != null) {
                    buffer = submit.uvMapping().wrap(buffer);
                }
            }

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
