package net.rubidium.interfaces;

import com.mojang.blaze3d.vertex.PoseStack;
import net.minecraft.client.resources.model.cuboid.ItemTransform;
import org.joml.Matrix4fc;
import org.joml.Vector3fc;

/** What an item layer's model bounding box depends on ({@code ItemStackRenderState.LayerRenderState}) */
public interface ItemLayerExtents {
    Vector3fc[] rubidium$extents();

    ItemTransform rubidium$itemTransform();

    Matrix4fc rubidium$localTransform();

    void rubidium$applyTransform(PoseStack.Pose pose);
}
