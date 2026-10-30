package net.ruthenium.render.entity;

import com.mojang.blaze3d.systems.RenderSystem;
import net.minecraft.core.BlockPos;
import net.minecraft.util.Mth;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.phys.Vec3;

/**
 * {@code EntityRenderer.getPackedLightCoords} without its two allocations per entity and frame
 * ({@code noAllocLightProbe}): {@code BlockPos.containing(entity.getLightProbePosition(partialTick))} builds the eye
 * position as a new {@code Vec3} and its block as a new {@code BlockPos}. For entity classes that keep
 * {@code Entity.getLightProbePosition} (the eye position, {@code getEyePosition}), {@link #probe} computes the same
 * doubles in the same order ({@code Mth.lerp(partialTick, xo, getX())}, the same for y plus the eye height, and z) and
 * hands back a marker instead of the vector; {@link #block} then floors them, as {@code BlockPos.containing} does, into
 * a scratch mutable position that the renderer's light lookups get instead of a new one. Render thread only.
 */
public final class EntityLightProbe {
    /** Stands for the computed probe position between the two calls; never read as a vector */
    public static final Vec3 MARKER = new Vec3(Double.NaN, Double.NaN, Double.NaN);

    private static final ClassValue<Boolean> DEFAULT_PROBE = new ClassValue<>() {
        @Override
        protected Boolean computeValue(Class<?> type) {
            try {
                return type.getMethod("getLightProbePosition", float.class).getDeclaringClass() == Entity.class;
            } catch (NoSuchMethodException e) {
                return false;
            }
        }
    };

    private static final BlockPos.MutableBlockPos BLOCK = new BlockPos.MutableBlockPos();
    private static double x, y, z;
    private static boolean pending;

    private EntityLightProbe() {
    }

    /** {@code entity.getLightProbePosition(partialTick)}, or {@link #MARKER} with the position kept for {@link #block} */
    public static Vec3 probe(Entity entity, float partialTick) {
        if (!RenderSystem.isOnRenderThread() || !DEFAULT_PROBE.get(entity.getClass())) {
            return entity.getLightProbePosition(partialTick);
        }
        // Entity.getEyePosition(partialTick)
        x = Mth.lerp((double) partialTick, entity.xo, entity.getX());
        y = Mth.lerp((double) partialTick, entity.yo, entity.getY()) + (double) entity.getEyeHeight();
        z = Mth.lerp((double) partialTick, entity.zo, entity.getZ());
        pending = true;
        return MARKER;
    }

    /** {@code BlockPos.containing(position)}; for {@link #MARKER} the kept position's block in a scratch position */
    public static boolean isMarker(Object position) {
        return position == MARKER && pending;
    }

    public static BlockPos block() {
        pending = false;
        return BLOCK.set(Mth.floor(x), Mth.floor(y), Mth.floor(z));
    }
}
