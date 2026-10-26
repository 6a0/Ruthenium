package net.vulkanmod.mixin.render.feature;

import com.mojang.blaze3d.vertex.PoseStack;
import com.mojang.blaze3d.vertex.SheetedDecalTextureGenerator;
import com.mojang.blaze3d.vertex.VertexConsumer;
import net.minecraft.client.model.Model;
import net.minecraft.client.renderer.feature.ModelFeatureRenderer;
import net.minecraft.client.renderer.feature.RenderTypeFeatureRenderer;
import net.vulkanmod.render.entity.ModelCulling;
import net.vulkanmod.render.entity.ModelGeometryCache;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Overwrite;
import org.spongepowered.asm.mixin.Shadow;

@Mixin(ModelFeatureRenderer.class)
public abstract class ModelFeatureRendererM extends RenderTypeFeatureRenderer<ModelFeatureRenderer.Submit<?>> {
    @Shadow @Final private PoseStack poseStack;

    /**
     * @author VulkanMod
     * @reason Back face culling ({@link ModelCulling}) and cached rest pose and model space geometry
     * ({@link ModelGeometryCache});
     * otherwise the vanilla body
     */
    @Overwrite
    private <S> void prepareModel(ModelFeatureRenderer.Submit<S> submit) {
        ModelCulling.beginModel(submit.renderType());
        try {
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

            model.setupAnim(submit.state());
            model.renderToBuffer(this.poseStack, buffer, submit.lightCoords(), submit.overlayCoords(), submit.tintedColor());
        } finally {
            ModelCulling.endModel();
        }
    }
}
