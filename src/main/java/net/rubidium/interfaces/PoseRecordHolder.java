package net.rubidium.interfaces;

import net.rubidium.render.entity.ModelPoseReplay;
import org.jspecify.annotations.Nullable;

/** The model submits of a render state in the current frame ({@code ModelPoseReplay}), two slots */
public interface PoseRecordHolder {
    ModelPoseReplay.@Nullable Record rubidium$getPoseRecord(int slot);

    void rubidium$setPoseRecord(int slot, ModelPoseReplay.Record record);
}
