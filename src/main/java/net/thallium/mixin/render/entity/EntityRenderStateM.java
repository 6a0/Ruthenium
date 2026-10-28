package net.thallium.mixin.render.entity;

import net.minecraft.client.renderer.entity.state.EntityRenderState;
import net.thallium.interfaces.PoseRecordHolder;
import net.thallium.render.entity.ModelPoseReplay;
import org.jspecify.annotations.Nullable;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Unique;

@Mixin(EntityRenderState.class)
public class EntityRenderStateM implements PoseRecordHolder {
    @Unique private ModelPoseReplay.@Nullable Record poseRecord0;
    @Unique private ModelPoseReplay.@Nullable Record poseRecord1;

    @Override
    public ModelPoseReplay.@Nullable Record thallium$getPoseRecord(int slot) {
        return slot == 0 ? this.poseRecord0 : this.poseRecord1;
    }

    @Override
    public void thallium$setPoseRecord(int slot, ModelPoseReplay.Record record) {
        if (slot == 0) {
            this.poseRecord0 = record;
        } else {
            this.poseRecord1 = record;
        }
    }
}
