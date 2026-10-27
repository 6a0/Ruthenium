package net.vulkanmod.mixin.render.entity.model;

import com.mojang.blaze3d.vertex.PoseStack;
import com.mojang.blaze3d.vertex.VertexConsumer;
import net.minecraft.client.model.geom.ModelPart;
import net.vulkanmod.interfaces.InstancedPartHolder;
import net.vulkanmod.render.entity.InstancedParts;
import org.jspecify.annotations.Nullable;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Overwrite;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.Unique;

import java.util.List;

/** Adds the part as an instance ({@link InstancedParts}) instead of writing its cubes when the model allows it */
@Mixin(ModelPart.class)
public class ModelPartM implements InstancedPartHolder {
    @Shadow @Final private List<ModelPart.Cube> cubes;

    @Unique private InstancedParts.@Nullable PartBatch batch;

    /**
     * @author VulkanMod
     * @reason Instanced parts; otherwise the vanilla body
     */
    @Overwrite
    private void compile(PoseStack.Pose pose, VertexConsumer builder, int lightCoords, int overlayCoords, int color) {
        if (InstancedParts.addPart((ModelPart) (Object) this, this.cubes, pose, lightCoords, overlayCoords, color)) {
            return;
        }
        for (ModelPart.Cube cube : this.cubes) {
            cube.compile(pose, builder, lightCoords, overlayCoords, color);
        }
    }

    @Override
    public InstancedParts.@Nullable PartBatch vulkanmod$getBatch() {
        return this.batch;
    }

    @Override
    public void vulkanmod$setBatch(InstancedParts.PartBatch batch) {
        this.batch = batch;
    }
}
