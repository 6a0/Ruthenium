package net.thallium.interfaces;

import net.thallium.render.entity.ModelPoseReplay;
import org.jspecify.annotations.Nullable;

/** The model submits of a render state in the current frame ({@code ModelPoseReplay}), two slots */
public interface PoseRecordHolder {
    ModelPoseReplay.@Nullable Record thallium$getPoseRecord(int slot);

    void thallium$setPoseRecord(int slot, ModelPoseReplay.Record record);
}
