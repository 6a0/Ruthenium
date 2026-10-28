package net.vulkanmod.interfaces;

import net.vulkanmod.render.entity.ModelPoseReplay;
import org.jspecify.annotations.Nullable;

/** The model submits of a render state in the current frame ({@code ModelPoseReplay}), two slots */
public interface PoseRecordHolder {
    ModelPoseReplay.@Nullable Record vulkanmod$getPoseRecord(int slot);

    void vulkanmod$setPoseRecord(int slot, ModelPoseReplay.Record record);
}
