package net.ruthenium.interfaces;

import com.mojang.blaze3d.vertex.PoseStack;
import com.mojang.blaze3d.vertex.VertexConsumer;

/** Writes a model part's cubes with a given pose, as the part tree does ({@code ModelPart.compile}) */
public interface ReplayablePart {
    void ruthenium$compile(PoseStack.Pose pose, VertexConsumer builder, int lightCoords, int overlayCoords, int color);
}
