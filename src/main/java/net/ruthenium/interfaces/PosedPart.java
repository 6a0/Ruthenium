package net.ruthenium.interfaces;

import net.minecraft.client.model.geom.ModelPart;
import net.ruthenium.render.entity.PlayerPoseDraws;
import org.jspecify.annotations.Nullable;

import java.util.List;

/** A model part's place in a {@link PlayerPoseDraws} layout */
public interface PosedPart {
    @Nullable Object ruthenium$poseLayout();

    int ruthenium$poseSlot();

    void ruthenium$setPoseLayout(@Nullable Object layout, int slot);

    List<ModelPart.Cube> ruthenium$cubes();
}
