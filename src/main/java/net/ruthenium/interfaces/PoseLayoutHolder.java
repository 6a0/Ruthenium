package net.ruthenium.interfaces;

import net.ruthenium.render.entity.PlayerPoseDraws;
import org.jspecify.annotations.Nullable;

/** A model's {@link PlayerPoseDraws} layout */
public interface PoseLayoutHolder {
    @Nullable Object ruthenium$poseLayout();

    void ruthenium$setPoseLayout(@Nullable Object layout);
}
