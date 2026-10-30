package net.ruthenium.mixin.render.entity.model;

import com.mojang.blaze3d.systems.RenderSystem;
import com.mojang.blaze3d.vertex.PoseStack;
import com.mojang.blaze3d.vertex.VertexConsumer;
import net.minecraft.client.model.geom.ModelPart;
import net.ruthenium.Initializer;
import net.ruthenium.interfaces.InstancedPartHolder;
import net.ruthenium.interfaces.PosedPart;
import net.ruthenium.interfaces.ReplayablePart;
import net.ruthenium.render.entity.InstancedParts;
import net.ruthenium.render.entity.LazyVertexBuilder;
import net.ruthenium.render.entity.ModelPoseReplay;
import net.ruthenium.render.entity.PlayerPoseDraws;
import org.joml.Quaternionf;
import org.jspecify.annotations.Nullable;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Overwrite;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import java.util.List;
import java.util.Map;

/**
 * Adds the part as an instance ({@link InstancedParts}) or hands its pose to a posed player draw ({@link PlayerPoseDraws})
 * instead of writing its cubes when the model allows it, records its pose for layers ({@link ModelPoseReplay}), and
 * keeps the quaternion of its rotation while its angles stay the same. With {@code partChildArrays} its children are
 * rendered from an array taken at construction instead of an iterator of the children map per render.
 */
@Mixin(ModelPart.class)
public class ModelPartM implements InstancedPartHolder, ReplayablePart, PosedPart {
    @Shadow @Final private List<ModelPart.Cube> cubes;
    @Shadow @Final private Map<String, ModelPart> children;
    @Shadow public boolean visible;
    @Shadow public boolean skipDraw;
    @Shadow public float x;
    @Shadow public float y;
    @Shadow public float z;
    @Shadow public float xRot;
    @Shadow public float yRot;
    @Shadow public float zRot;
    @Shadow public float xScale;
    @Shadow public float yScale;
    @Shadow public float zScale;

    @Unique private InstancedParts.@Nullable PartBatches batches;
    /** The rotation of the angles below, built on the render thread (models are shared by all entities of a type) */
    @Unique private @Nullable Quaternionf rotation;
    @Unique private int rotationXBits;
    @Unique private int rotationYBits;
    @Unique private int rotationZBits;
    /** The part's {@link PlayerPoseDraws} layout and slot in it */
    @Unique private @Nullable Object poseLayout;
    @Unique private int poseSlot;
    /** The children in the map's order, taken at construction (vanilla never changes the map of a part) */
    @Unique private ModelPart @Nullable [] childArray;

    @Inject(method = "<init>", at = @At("RETURN"))
    private void keepChildArray(List<ModelPart.Cube> cubes, Map<String, ModelPart> children, CallbackInfo ci) {
        this.childArray = children.values().toArray(new ModelPart[0]);
    }

    /**
     * @author Ruthenium
     * @reason Vanilla's body, except that the children come from an array taken at construction instead of an iterator
     * of the children map (allocated for every part rendered, leaves included, as every baked part has a map) while the
     * map's size still matches it
     */
    @Overwrite
    public void render(final PoseStack poseStack, final VertexConsumer buffer, final int lightCoords, final int overlayCoords, final int color) {
        if (this.visible) {
            if (!this.cubes.isEmpty() || !this.children.isEmpty()) {
                poseStack.pushPose();
                ((ModelPart) (Object) this).translateAndRotate(poseStack);
                if (!this.skipDraw) {
                    this.compile(poseStack.last(), buffer, lightCoords, overlayCoords, color);
                }

                ModelPart[] childArray = this.childArray;
                if (Initializer.CONFIG.partChildArrays && childArray != null && childArray.length == this.children.size()) {
                    for (ModelPart child : childArray) {
                        child.render(poseStack, buffer, lightCoords, overlayCoords, color);
                    }
                } else {
                    for (ModelPart child : this.children.values()) {
                        child.render(poseStack, buffer, lightCoords, overlayCoords, color);
                    }
                }

                poseStack.popPose();
            }
        }
    }

    /**
     * @author Ruthenium
     * @reason The rotation's quaternion (three sines and three cosines) is kept while the part's angles stay the same,
     * which they do for parts at rest across the entities that share the model; the same quaternion, so the same pose
     */
    @Overwrite
    public void translateAndRotate(final PoseStack poseStack) {
        poseStack.translate(this.x / 16.0F, this.y / 16.0F, this.z / 16.0F);
        if (this.xRot != 0.0F || this.yRot != 0.0F || this.zRot != 0.0F) {
            poseStack.rotate(this.rotation());
        }

        if (this.xScale != 1.0F || this.yScale != 1.0F || this.zScale != 1.0F) {
            poseStack.scale(this.xScale, this.yScale, this.zScale);
        }
    }

    @Unique
    private Quaternionf rotation() {
        if (!RenderSystem.isOnRenderThread()) {
            return new Quaternionf().rotationZYX(this.zRot, this.yRot, this.xRot);
        }
        int xBits = Float.floatToRawIntBits(this.xRot);
        int yBits = Float.floatToRawIntBits(this.yRot);
        int zBits = Float.floatToRawIntBits(this.zRot);
        Quaternionf rotation = this.rotation;
        if (rotation == null) {
            rotation = this.rotation = new Quaternionf();
        } else if (xBits == this.rotationXBits && yBits == this.rotationYBits && zBits == this.rotationZBits) {
            return rotation;
        }
        this.rotationXBits = xBits;
        this.rotationYBits = yBits;
        this.rotationZBits = zBits;
        return rotation.rotationZYX(this.zRot, this.yRot, this.xRot);
    }

    /**
     * @author Ruthenium
     * @reason Instanced parts and posed player draws; otherwise the vanilla body
     */
    @Overwrite
    private void compile(PoseStack.Pose pose, VertexConsumer builder, int lightCoords, int overlayCoords, int color) {
        if (ModelPoseReplay.isRecording() && !this.cubes.isEmpty()) {
            ModelPoseReplay.record((ModelPart) (Object) this, pose);
        }
        if (PlayerPoseDraws.isCapturing()) {
            if (!this.cubes.isEmpty()) {
                PlayerPoseDraws.capturePart((ModelPart) (Object) this, this.cubes, pose);
            }
            return;
        }
        if (InstancedParts.addPart((ModelPart) (Object) this, this.cubes, pose, lightCoords, overlayCoords, color)) {
            return;
        }
        if (builder instanceof LazyVertexBuilder lazy) {
            builder = lazy.resolve();
        }
        for (ModelPart.Cube cube : this.cubes) {
            cube.compile(pose, builder, lightCoords, overlayCoords, color);
        }
    }

    @Override
    public void ruthenium$compile(PoseStack.Pose pose, VertexConsumer builder, int lightCoords, int overlayCoords, int color) {
        this.compile(pose, builder, lightCoords, overlayCoords, color);
    }

    @Override
    public InstancedParts.@Nullable PartBatches ruthenium$getBatches() {
        return this.batches;
    }

    @Override
    public void ruthenium$setBatches(InstancedParts.PartBatches batches) {
        this.batches = batches;
    }

    @Override
    public @Nullable Object ruthenium$poseLayout() {
        return this.poseLayout;
    }

    @Override
    public int ruthenium$poseSlot() {
        return this.poseSlot;
    }

    @Override
    public void ruthenium$setPoseLayout(@Nullable Object layout, int slot) {
        this.poseLayout = layout;
        this.poseSlot = slot;
    }

    @Override
    public List<ModelPart.Cube> ruthenium$cubes() {
        return this.cubes;
    }
}
