package net.ruthenium.interfaces;

import com.mojang.blaze3d.vertex.PoseStack;
import net.minecraft.client.resources.model.cuboid.ItemTransform;
import org.joml.Matrix4fc;
import org.joml.Vector3fc;

/** What an item layer's model bounding box depends on ({@code ItemStackRenderState.LayerRenderState}) */
public interface ItemLayerExtents {
    Vector3fc[] ruthenium$extents();

    ItemTransform ruthenium$itemTransform();

    Matrix4fc ruthenium$localTransform();

    void ruthenium$applyTransform(PoseStack.Pose pose);
}
