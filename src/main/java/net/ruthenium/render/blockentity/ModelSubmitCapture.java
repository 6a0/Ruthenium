package net.ruthenium.render.blockentity;

import com.mojang.blaze3d.vertex.PoseStack;
import net.minecraft.client.gui.Font;
import net.minecraft.client.model.Model;
import net.minecraft.client.renderer.OrderedSubmitNodeCollector;
import net.minecraft.client.renderer.SubmitNodeCollector;
import net.minecraft.client.renderer.block.MovingBlockRenderState;
import net.minecraft.client.renderer.block.dispatch.BlockStateModelPart;
import net.minecraft.client.renderer.entity.state.EntityRenderState;
import net.minecraft.client.renderer.feature.ModelFeatureRenderer;
import net.minecraft.client.renderer.gizmos.DrawableGizmoPrimitives;
import net.minecraft.client.renderer.item.ItemStackRenderState;
import net.minecraft.client.renderer.rendertype.RenderType;
import net.minecraft.client.renderer.state.level.CameraRenderState;
import net.minecraft.client.renderer.state.level.QuadParticleRenderState;
import net.minecraft.client.renderer.texture.UvMapping;
import net.minecraft.client.resources.model.geometry.ItemQuads;
import net.minecraft.network.chat.Component;
import net.minecraft.util.FormattedCharSequence;
import net.minecraft.world.item.ItemDisplayContext;
import net.minecraft.world.phys.Vec3;
import net.minecraft.world.phys.shapes.VoxelShape;
import org.joml.Matrix4f;
import org.joml.Quaternionf;
import org.jspecify.annotations.Nullable;

import java.util.ArrayList;
import java.util.List;

/**
 * Records what a block entity renderer submits, for {@link RetainedBlockEntities} and {@link InstancedModels}. Only
 * model submits (and model parts, which vanilla submits as models) and text are accepted; anything else marks the
 * capture as unsupported.
 */
final class ModelSubmitCapture implements SubmitNodeCollector {
    static final class ModelRecord {
        @Nullable Model<?> model;
        @Nullable Object state;
        final PoseStack.Pose pose = new PoseStack().last();
        @Nullable RenderType renderType;
        int lightCoords;
        int overlayCoords;
        int tintedColor;
        @Nullable UvMapping uvMapping;
        int outlineColor;
    }

    static final class TextRecord {
        final Matrix4f pose = new Matrix4f();
        float x, y;
        @Nullable FormattedCharSequence string;
        boolean dropShadow;
        Font.@Nullable DisplayMode displayMode;
        int lightCoords;
        int color;
        int backgroundColor;
        int outlineColor;
    }

    private final List<ModelRecord> records = new ArrayList<>();
    private final List<TextRecord> texts = new ArrayList<>();
    int modelCount;
    int textCount;
    boolean unsupported;

    void reset() {
        for (int i = 0; i < this.modelCount; i++) {
            ModelRecord record = this.records.get(i);
            record.model = null;
            record.state = null;
            record.renderType = null;
            record.uvMapping = null;
        }
        for (int i = 0; i < this.textCount; i++) {
            this.texts.get(i).string = null;
        }
        this.modelCount = 0;
        this.textCount = 0;
        this.unsupported = false;
    }

    /** Exactly one model was submitted and nothing else */
    boolean isSingleModel() {
        return !this.unsupported && this.modelCount == 1 && this.textCount == 0;
    }

    /** At least one model or text was submitted and nothing else */
    boolean isModelsAndText() {
        return !this.unsupported && this.modelCount + this.textCount > 0;
    }

    /** The i-th submitted model, valid until the next {@link #reset} */
    ModelRecord model(int i) {
        return this.records.get(i);
    }

    /** The i-th submitted text, valid until the next {@link #reset} */
    TextRecord text(int i) {
        return this.texts.get(i);
    }

    @Override
    public OrderedSubmitNodeCollector order(int order) {
        // Further orders (crumbling overlays etc.) can't be retained
        if (order != 0) {
            this.unsupported = true;
        }
        return this;
    }

    @Override
    public <S> void submitModel(Model<? super S> model, S state, PoseStack poseStack, RenderType renderType, int lightCoords,
                                int overlayCoords, int tintedColor, @Nullable UvMapping uvMapping, int outlineColor) {
        if (this.modelCount == this.records.size()) {
            this.records.add(new ModelRecord());
        }
        ModelRecord record = this.records.get(this.modelCount++);
        record.model = model;
        record.state = state;
        record.pose.set(poseStack.last());
        record.renderType = renderType;
        record.lightCoords = lightCoords;
        record.overlayCoords = overlayCoords;
        record.tintedColor = tintedColor;
        record.uvMapping = uvMapping;
        record.outlineColor = outlineColor;
    }

    @Override
    public <S> void submitCrumblingOverlay(Model<? super S> model, S state, PoseStack poseStack, RenderType renderType, int lightCoords,
                                           int overlayCoords, int tintedColor, ModelFeatureRenderer.CrumblingOverlay crumblingOverlay) {
        this.unsupported = true;
    }

    @Override
    public void submitShadow(PoseStack poseStack, float radius, List<EntityRenderState.ShadowPiece> pieces) {
        this.unsupported = true;
    }

    @Override
    public void submitNameTag(PoseStack poseStack, @Nullable Vec3 nameTagAttachment, int offset, Component name, boolean seeThrough,
                              int lightCoords, CameraRenderState camera) {
        this.unsupported = true;
    }

    @Override
    public void submitText(PoseStack poseStack, float x, float y, FormattedCharSequence string, boolean dropShadow,
                           Font.DisplayMode displayMode, int lightCoords, int color, int backgroundColor, int outlineColor) {
        if (this.textCount == this.texts.size()) {
            this.texts.add(new TextRecord());
        }
        TextRecord record = this.texts.get(this.textCount++);
        record.pose.set(poseStack.last().pose());
        record.x = x;
        record.y = y;
        record.string = string;
        record.dropShadow = dropShadow;
        record.displayMode = displayMode;
        record.lightCoords = lightCoords;
        record.color = color;
        record.backgroundColor = backgroundColor;
        record.outlineColor = outlineColor;
    }

    @Override
    public void submitTextBackground(PoseStack poseStack, float x0, float y0, float x1, float y1, int color,
                                     Font.DisplayMode displayMode, int lightCoords) {
        this.unsupported = true;
    }

    @Override
    public void submitFlame(PoseStack poseStack, EntityRenderState renderState, Quaternionf rotation) {
        this.unsupported = true;
    }

    @Override
    public void submitLeash(PoseStack poseStack, EntityRenderState.LeashState leashState) {
        this.unsupported = true;
    }

    @Override
    public void submitMovingBlock(PoseStack poseStack, MovingBlockRenderState movingBlockRenderState, int outlineColor) {
        this.unsupported = true;
    }

    @Override
    public void submitBlockModel(PoseStack poseStack, RenderType renderType, List<BlockStateModelPart> parts, int[] tintLayers,
                                 int lightCoords, int overlayCoords, int outlineColor) {
        this.unsupported = true;
    }

    @Override
    public void submitBreakingBlockModel(PoseStack poseStack, List<BlockStateModelPart> parts, int progress, boolean isBlockTranslucent) {
        this.unsupported = true;
    }

    @Override
    public void submitShapeOutline(PoseStack poseStack, VoxelShape shape, RenderType renderType, int color, float width, boolean afterTerrain) {
        this.unsupported = true;
    }

    @Override
    public void submitItem(PoseStack poseStack, ItemDisplayContext displayContext, int lightCoords, int overlayCoords, int outlineColor,
                           int[] tintLayers, ItemQuads quads, ItemStackRenderState.FoilType foilType) {
        this.unsupported = true;
    }

    @Override
    public void submitCustomGeometry(PoseStack poseStack, RenderType renderType, CustomGeometryRenderer customGeometryRenderer) {
        this.unsupported = true;
    }

    @Override
    public void submitQuadParticleGroup(QuadParticleRenderState particles) {
        this.unsupported = true;
    }

    @Override
    public void submitGizmoPrimitives(DrawableGizmoPrimitives.Group group, CameraRenderState camera, boolean onTop) {
        this.unsupported = true;
    }
}
