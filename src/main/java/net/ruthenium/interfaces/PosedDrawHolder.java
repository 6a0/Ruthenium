package net.ruthenium.interfaces;

import com.mojang.blaze3d.vertex.VertexSorting;
import net.ruthenium.render.entity.PlayerPoseDraws;
import org.jspecify.annotations.Nullable;

/** A {@code StagedVertexBuffer.Draw} that {@link PlayerPoseDraws} draws instead of its (empty) vertices */
public interface PosedDrawHolder {
    PlayerPoseDraws.@Nullable PoseDraw ruthenium$poseDraw();

    void ruthenium$setPoseDraw(PlayerPoseDraws.@Nullable PoseDraw draw);

    @Nullable VertexSorting ruthenium$quadSorting();
}
