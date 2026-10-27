package net.vulkanmod.mixin.render.item;

import com.mojang.blaze3d.vertex.PoseStack;
import net.minecraft.client.renderer.item.ItemStackRenderState;
import net.minecraft.client.resources.model.cuboid.ItemTransform;
import net.vulkanmod.interfaces.ItemLayerExtents;
import org.joml.Matrix4f;
import org.joml.Matrix4fc;
import org.joml.Vector3fc;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;

import java.util.function.Supplier;

@Mixin(ItemStackRenderState.LayerRenderState.class)
public abstract class ItemStackRenderStateLayerM implements ItemLayerExtents {
    @Shadow private ItemTransform itemTransform;
    @Shadow @Final private Matrix4f localTransform;
    @Shadow private Supplier<Vector3fc[]> extents;

    @Shadow protected abstract void applyTransform(PoseStack.Pose localPose);

    @Override
    public Vector3fc[] vulkanmod$extents() {
        return this.extents.get();
    }

    @Override
    public ItemTransform vulkanmod$itemTransform() {
        return this.itemTransform;
    }

    @Override
    public Matrix4fc vulkanmod$localTransform() {
        return this.localTransform;
    }

    @Override
    public void vulkanmod$applyTransform(PoseStack.Pose pose) {
        this.applyTransform(pose);
    }
}
