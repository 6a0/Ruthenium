package net.ruthenium.render.blockentity;

import it.unimi.dsi.fastutil.objects.Reference2BooleanOpenHashMap;
import net.minecraft.client.renderer.culling.Frustum;
import net.minecraft.core.BlockPos;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.world.level.block.entity.BlockEntityType;
import net.ruthenium.mixin.render.blockentity.FrustumAccessor;

/**
 * Per block entity frustum culling.
 * <p>
 * There is no render bounding box for block entities, so a conservative box is used: the block plus one block on
 * every side, and two extra blocks above (standing banners reach ~2.6 blocks up). Only vanilla block entity types
 * are culled; modded ones may render arbitrarily large geometry from a single block, so they are left to vanilla's
 * per-section culling. Block entities that render off screen (beacons etc.) never reach this path.
 * <p>
 * A section is tested with the same margins around its 16³ blocks, so its box contains the box of every block
 * entity in it: when the section box is fully inside (or outside) the frustum, so is every block entity box.
 * Sections crossing the frustum are split the same way into 4³-block cells.
 */
public final class BlockEntityCulling {
    public static final int SECTION_UNKNOWN = 0;
    public static final int SECTION_INSIDE = 1;
    public static final int SECTION_OUTSIDE = 2;
    public static final int SECTION_PARTIAL = 3;
    public static final int CELL_SIZE = 4;

    private static final double MARGIN = 1.0;
    private static final double EXTRA_TOP = 2.0;

    private static final Reference2BooleanOpenHashMap<BlockEntityType<?>> CULLABLE_TYPES = new Reference2BooleanOpenHashMap<>();

    private BlockEntityCulling() {
    }

    public static int classifySection(Frustum frustum, BlockPos origin) {
        return classify(frustum, origin.getX(), origin.getY(), origin.getZ(), 16);
    }

    /** Classifies the {@link #CELL_SIZE}³ cell of a section that contains the block */
    public static int classifyCell(Frustum frustum, int x, int y, int z) {
        int mask = -CELL_SIZE;
        return classify(frustum, x & mask, y & mask, z & mask, CELL_SIZE);
    }

    /** Index of the cell of the block within its section, 0..63 */
    public static int cellIndex(int x, int y, int z) {
        return (x >> 2 & 3) | (y >> 2 & 3) << 2 | (z >> 2 & 3) << 4;
    }

    private static int classify(Frustum frustum, int x, int y, int z, int size) {
        int result = cubeInFrustum(frustum, x, y, z, size);
        return switch (result) {
            case -2 -> SECTION_INSIDE;
            case -1 -> SECTION_PARTIAL;
            default -> SECTION_OUTSIDE;
        };
    }

    public static boolean isOutsideFrustum(Frustum frustum, int x, int y, int z) {
        int result = cubeInFrustum(frustum, x, y, z, 1);
        return result != -2 && result != -1;
    }

    private static int cubeInFrustum(Frustum frustum, int x, int y, int z, int size) {
        return ((FrustumAccessor) frustum).invokeCubeInFrustum(
                x - MARGIN, y - MARGIN, z - MARGIN,
                x + size + MARGIN, y + size + MARGIN + EXTRA_TOP, z + size + MARGIN);
    }

    public static boolean isCullable(BlockEntityType<?> type) {
        // Only accessed from the render thread
        if (CULLABLE_TYPES.containsKey(type)) {
            return CULLABLE_TYPES.getBoolean(type);
        }

        Identifier id = BuiltInRegistries.BLOCK_ENTITY_TYPE.getKey(type);
        boolean cullable = id != null && id.getNamespace().equals(Identifier.DEFAULT_NAMESPACE);
        CULLABLE_TYPES.put(type, cullable);
        return cullable;
    }
}
