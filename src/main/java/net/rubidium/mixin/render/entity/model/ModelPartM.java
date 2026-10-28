package net.rubidium.mixin.render.entity.model;

import com.mojang.blaze3d.systems.RenderSystem;
import com.mojang.blaze3d.vertex.PoseStack;
import com.mojang.blaze3d.vertex.VertexConsumer;
import net.minecraft.client.model.geom.ModelPart;
import net.rubidium.interfaces.InstancedPartHolder;
import net.rubidium.interfaces.ReplayablePart;
import net.rubidium.render.entity.InstancedParts;
import net.rubidium.render.entity.LazyVertexBuilder;
import net.rubidium.render.entity.ModelPoseReplay;
import org.joml.Quaternionf;
import org.jspecify.annotations.Nullable;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Overwrite;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.Unique;

import java.util.List;

/**
 * Adds the part as an instance ({@link InstancedParts}) instead of writing its cubes when the model allows it, records
 * its pose for layers ({@link ModelPoseReplay}), and keeps the quaternion of its rotation while its angles stay the
 * same.
 */
@Mixin(ModelPart.class)
public class ModelPartM implements InstancedPartHolder, ReplayablePart {
    @Shadow @Final private List<ModelPart.Cube> cubes;
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

    /**
     * @author Rubidium
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
     * @author Rubidium
     * @reason Instanced parts; otherwise the vanilla body
     */
    @Overwrite
    private void compile(PoseStack.Pose pose, VertexConsumer builder, int lightCoords, int overlayCoords, int color) {
        if (ModelPoseReplay.isRecording() && !this.cubes.isEmpty()) {
            ModelPoseReplay.record((ModelPart) (Object) this, pose);
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
    public void rubidium$compile(PoseStack.Pose pose, VertexConsumer builder, int lightCoords, int overlayCoords, int color) {
        this.compile(pose, builder, lightCoords, overlayCoords, color);
    }

    @Override
    public InstancedParts.@Nullable PartBatches rubidium$getBatches() {
        return this.batches;
    }

    @Override
    public void rubidium$setBatches(InstancedParts.PartBatches batches) {
        this.batches = batches;
    }
}
