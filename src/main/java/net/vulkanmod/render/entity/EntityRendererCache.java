package net.vulkanmod.render.entity;

import com.google.common.collect.ImmutableMap;
import net.minecraft.client.renderer.culling.Frustum;
import net.minecraft.client.renderer.entity.EntityRenderer;
import net.minecraft.client.renderer.entity.LivingEntityRenderer;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.entity.Leashable;
import net.minecraft.world.phys.AABB;
import net.vulkanmod.interfaces.EntityRendererHolder;
import net.vulkanmod.mixin.render.blockentity.FrustumAccessor;
import org.jspecify.annotations.Nullable;

import java.lang.reflect.Method;
import java.util.Map;

/**
 * Entity renderers kept on their {@link EntityType}, and a cheaper exact entity frustum test built on them.
 * <p>
 * {@code EntityRenderDispatcher.getRenderer} looks the renderer of a non-player entity up in the dispatcher's renderer
 * map (a Guava {@code ImmutableMap}, replaced on resource reload) up to three times per entity and frame (culling,
 * extraction, submission). The entry kept on the type holds the map it came from, so it is used only while the
 * dispatcher still holds that map: the same renderer the lookup would return.
 * <p>
 * The frustum test ({@link #isVisible}) answers {@code EntityRenderer.shouldRender} for renderers that inherit its
 * culling unchanged (no override of {@code shouldRender}, {@code affectedByCulling} or {@code getBoundingBoxForCulling}
 * below {@code EntityRenderer}, or below {@code LivingEntityRenderer} for living ones), with the same calls in the same
 * order, but without allocating the inflated box or looking the renderer up. {@code LivingEntityRenderer} widens the box
 * by another half block for an entity wearing a dragon head; instead of reading the head slot, the box is tested with
 * that widening: fully inside the frustum means visible either way, outside means hidden either way, and only when it
 * crosses the frustum's edge is the box without the widening tested: visible means visible either way, and an entity
 * in between (within half a block of the edge) takes vanilla's path. So do entities with a
 * leash holder (vanilla tests the holder's box too), boxes with non-finite coordinates (vanilla substitutes a box around
 * the position) and any renderer not known to be plain. The frustum test is monotone in the box, and the widened
 * coordinates are computed in the same order as vanilla's {@code inflate} calls, so the answers are vanilla's.
 */
public final class EntityRendererCache {
    public static final int CULL_VANILLA = 0;
    public static final int CULL_PLAIN = 1;
    public static final int CULL_LIVING = 2;

    /** Answers of {@link #isVisible} */
    public static final int HIDDEN = 0;
    public static final int VISIBLE = 1;
    public static final int UNDECIDED = -1;

    public record Entry(Map<?, ?> renderers, EntityRenderer<?, ?> renderer, int cullKind) {
    }

    private EntityRendererCache() {
    }

    /** Whether lookups in this map may be kept on the key (immutable: entries never change while the map is held) */
    public static boolean cacheable(Map<?, ?> renderers, Object key) {
        return key instanceof EntityRendererHolder && renderers instanceof ImmutableMap<?, ?>;
    }

    /** The renderer kept on the key for this map, or null */
    public static @Nullable Object cached(Map<?, ?> renderers, Object key) {
        Entry entry = ((EntityRendererHolder) key).vulkanmod$rendererEntry();
        return entry != null && entry.renderers == renderers ? entry.renderer : null;
    }

    /** Keeps the result of {@code renderers.get(key)} on the key */
    public static void store(Map<?, ?> renderers, Object key, @Nullable Object renderer) {
        if (renderer instanceof EntityRenderer<?, ?> entityRenderer) {
            ((EntityRendererHolder) key).vulkanmod$setRendererEntry(new Entry(renderers, entityRenderer, cullKind(entityRenderer.getClass())));
        }
    }

    /** The entry of an entity type if it belongs to the given renderer map */
    public static @Nullable Entry entry(EntityType<?> type, Map<?, ?> renderers) {
        Entry entry = ((EntityRendererHolder) type).vulkanmod$rendererEntry();
        return entry != null && entry.renderers == renderers ? entry : null;
    }

    private static int cullKind(Class<?> rendererClass) {
        boolean living = LivingEntityRenderer.class.isAssignableFrom(rendererClass);
        Class<?> base = living ? LivingEntityRenderer.class : EntityRenderer.class;
        for (Class<?> c = rendererClass; c != base && c != null; c = c.getSuperclass()) {
            for (Method method : c.getDeclaredMethods()) {
                String name = method.getName();
                if (name.equals("shouldRender") || name.equals("affectedByCulling") || name.equals("getBoundingBoxForCulling")) {
                    return CULL_VANILLA;
                }
            }
        }
        return living ? CULL_LIVING : CULL_PLAIN;
    }

    /**
     * {@code EntityRenderer.shouldRender} of a renderer whose culling kind is {@link #CULL_PLAIN} or {@link #CULL_LIVING}:
     * {@link #VISIBLE}, {@link #HIDDEN}, or {@link #UNDECIDED} when vanilla's path has to answer.
     */
    public static int isVisible(int cullKind, Entity entity, Frustum frustum, double camX, double camY, double camZ, float partialTicks) {
        if (!entity.shouldRender(camX, camY, camZ)) {
            return HIDDEN;
        }

        AABB box = entity.getInterpolatedBoundingBox(partialTicks);
        double minX = box.minX, minY = box.minY, minZ = box.minZ;
        double maxX = box.maxX, maxY = box.maxY, maxZ = box.maxZ;
        if (!Double.isFinite(minX) || !Double.isFinite(minY) || !Double.isFinite(minZ)
                || !Double.isFinite(maxX) || !Double.isFinite(maxY) || !Double.isFinite(maxZ)) {
            return UNDECIDED;
        }

        FrustumAccessor accessor = (FrustumAccessor) frustum;
        int result;
        if (cullKind == CULL_LIVING) {
            // The box of an entity wearing a dragon head (inflate(0.5) by the renderer, then inflate(0.5) for culling)
            // contains the one without: fully inside means both are visible, outside means both are hidden
            result = accessor.invokeCubeInFrustum(minX - 0.5 - 0.5, minY - 0.5 - 0.5, minZ - 0.5 - 0.5,
                    maxX + 0.5 + 0.5, maxY + 0.5 + 0.5, maxZ + 0.5 + 0.5);
            if (result == -2) {
                return VISIBLE;
            }
            if (result == -1) {
                result = accessor.invokeCubeInFrustum(minX - 0.5, minY - 0.5, minZ - 0.5, maxX + 0.5, maxY + 0.5, maxZ + 0.5);
                return result == -2 || result == -1 ? VISIBLE : UNDECIDED;
            }
        } else {
            result = accessor.invokeCubeInFrustum(minX - 0.5, minY - 0.5, minZ - 0.5, maxX + 0.5, maxY + 0.5, maxZ + 0.5);
            if (result == -2 || result == -1) {
                return VISIBLE;
            }
        }

        if (entity instanceof Leashable leashable && leashable.getLeashHolder() != null) {
            return UNDECIDED;
        }
        return HIDDEN;
    }
}
