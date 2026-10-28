package net.ruthenium.render.entity;

import com.mojang.blaze3d.systems.RenderSystem;
import com.mojang.blaze3d.vertex.PoseStack;
import net.minecraft.client.resources.model.cuboid.ItemTransform;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.Vec3;
import net.ruthenium.interfaces.ItemLayerExtents;
import org.joml.Matrix4f;
import org.joml.Matrix4fc;
import org.joml.Vector3f;
import org.joml.Vector3fc;
import org.jspecify.annotations.Nullable;

import java.util.IdentityHashMap;
import java.util.Map;

/**
 * The model bounding box of item render states ({@code ItemStackRenderState.getModelBoundingBox}), kept per layer
 * across frames. Vanilla caches it in the render state, which is extracted anew every frame, so every dropped item
 * transformed all the corner points of its model (every distinct quad vertex, hundreds for flat items) each frame.
 * <p>
 * A layer's box only depends on its model's points (an array the model keeps), its item transform (the model's, per
 * display context), the hand and its local transform, so it is kept per points array and checked against the rest.
 * The box of the render state is the union of its layers' boxes: the minimum and maximum of all points, as vanilla's
 * {@code AABB.Builder} takes them, in floats, so the result is the same.
 */
public final class ItemBoundingBoxes {
    /** Bounds the cache; models that change (reloads, mods) refill it */
    private static final int MAX_ENTRIES = 4096;

    private static final Map<Vector3fc[], Box> BOXES = new IdentityHashMap<>();
    private static int entries;
    private static final PoseStack.Pose POSE = new PoseStack.Pose();
    private static final Vector3f SCRATCH = new Vector3f();

    private ItemBoundingBoxes() {
    }

    private static final class Box {
        final ItemTransform itemTransform;
        final boolean leftHand;
        final Matrix4f localTransform;
        final float minX, minY, minZ, maxX, maxY, maxZ;
        final @Nullable Box next;

        Box(ItemTransform itemTransform, boolean leftHand, Matrix4fc localTransform, float[] bounds, @Nullable Box next) {
            this.itemTransform = itemTransform;
            this.leftHand = leftHand;
            this.localTransform = new Matrix4f(localTransform);
            this.minX = bounds[0];
            this.minY = bounds[1];
            this.minZ = bounds[2];
            this.maxX = bounds[3];
            this.maxY = bounds[4];
            this.maxZ = bounds[5];
            this.next = next;
        }
    }

    public static void clear() {
        BOXES.clear();
        entries = 0;
    }

    /** @return the bounding box of these layers, or null to compute it as vanilla does (off the render thread) */
    public static @Nullable AABB compute(Object[] layers, int count, boolean leftHand) {
        if (!RenderSystem.isOnRenderThread()) {
            return null;
        }

        float minX = Float.POSITIVE_INFINITY, minY = Float.POSITIVE_INFINITY, minZ = Float.POSITIVE_INFINITY;
        float maxX = Float.NEGATIVE_INFINITY, maxY = Float.NEGATIVE_INFINITY, maxZ = Float.NEGATIVE_INFINITY;
        boolean defined = false;
        for (int i = 0; i < count; i++) {
            ItemLayerExtents layer = (ItemLayerExtents) layers[i];
            Vector3fc[] extents = layer.ruthenium$extents();
            if (extents.length == 0) {
                continue;
            }
            Box box = find(layer, extents, leftHand);
            minX = Math.min(minX, box.minX);
            minY = Math.min(minY, box.minY);
            minZ = Math.min(minZ, box.minZ);
            maxX = Math.max(maxX, box.maxX);
            maxY = Math.max(maxY, box.maxY);
            maxZ = Math.max(maxZ, box.maxZ);
            defined = true;
        }
        // As getModelBoundingBox
        return defined ? new AABB(minX, minY, minZ, maxX, maxY, maxZ) : AABB.ofSize(Vec3.ZERO, 0.0, 0.0, 0.0);
    }

    private static Box find(ItemLayerExtents layer, Vector3fc[] extents, boolean leftHand) {
        ItemTransform itemTransform = layer.ruthenium$itemTransform();
        Matrix4fc localTransform = layer.ruthenium$localTransform();
        Box head = BOXES.get(extents);
        for (Box box = head; box != null; box = box.next) {
            if (box.itemTransform == itemTransform && box.leftHand == leftHand && box.localTransform.equals(localTransform)) {
                return box;
            }
        }

        // As visitExtents
        PoseStack.Pose pose = POSE;
        pose.setIdentity();
        layer.ruthenium$applyTransform(pose);
        Matrix4f transform = pose.pose();
        float[] bounds = {Float.POSITIVE_INFINITY, Float.POSITIVE_INFINITY, Float.POSITIVE_INFINITY,
                Float.NEGATIVE_INFINITY, Float.NEGATIVE_INFINITY, Float.NEGATIVE_INFINITY};
        for (Vector3fc extent : extents) {
            Vector3f v = SCRATCH.set(extent).mulPosition(transform);
            bounds[0] = Math.min(bounds[0], v.x());
            bounds[1] = Math.min(bounds[1], v.y());
            bounds[2] = Math.min(bounds[2], v.z());
            bounds[3] = Math.max(bounds[3], v.x());
            bounds[4] = Math.max(bounds[4], v.y());
            bounds[5] = Math.max(bounds[5], v.z());
        }

        if (entries >= MAX_ENTRIES) {
            clear();
            head = null;
        }
        Box box = new Box(itemTransform, leftHand, localTransform, bounds, head);
        BOXES.put(extents, box);
        entries++;
        return box;
    }
}
