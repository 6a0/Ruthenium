package net.ruthenium.mixin.render.item;

import net.minecraft.client.renderer.item.ItemStackRenderState;
import net.minecraft.world.item.ItemDisplayContext;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.Vec3;
import net.ruthenium.render.entity.ItemBoundingBoxes;
import org.joml.Vector3fc;
import org.jspecify.annotations.Nullable;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Overwrite;
import org.spongepowered.asm.mixin.Shadow;

import java.util.function.Consumer;

/** Model bounding boxes from {@link ItemBoundingBoxes} instead of transforming every model point each frame */
@Mixin(ItemStackRenderState.class)
public abstract class ItemStackRenderStateM {
    @Shadow ItemDisplayContext displayContext;
    @Shadow private int activeLayerCount;
    @Shadow private @Nullable AABB cachedModelBoundingBox;
    @Shadow private ItemStackRenderState.LayerRenderState[] layers;

    @Shadow public abstract void visitExtents(Consumer<Vector3fc> output);

    /**
     * @author Ruthenium
     * @reason Bounding boxes kept per layer; otherwise the vanilla body
     */
    @Overwrite
    public AABB getModelBoundingBox() {
        if (this.cachedModelBoundingBox != null) {
            return this.cachedModelBoundingBox;
        }

        AABB aabb = ItemBoundingBoxes.compute(this.layers, this.activeLayerCount, this.displayContext.leftHand());
        if (aabb == null) {
            AABB.Builder collector = new AABB.Builder();
            this.visitExtents(collector::include);
            aabb = collector.isDefined() ? collector.build() : AABB.ofSize(Vec3.ZERO, 0.0, 0.0, 0.0);
        }
        this.cachedModelBoundingBox = aabb;
        return aabb;
    }
}
