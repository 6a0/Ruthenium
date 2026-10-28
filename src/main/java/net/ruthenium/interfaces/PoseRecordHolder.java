package net.ruthenium.interfaces;

import net.ruthenium.render.entity.ModelPoseReplay;
import org.jspecify.annotations.Nullable;

/** The model submits of a render state in the current frame ({@code ModelPoseReplay}), two slots */
public interface PoseRecordHolder {
    ModelPoseReplay.@Nullable Record ruthenium$getPoseRecord(int slot);

    void ruthenium$setPoseRecord(int slot, ModelPoseReplay.Record record);
}
