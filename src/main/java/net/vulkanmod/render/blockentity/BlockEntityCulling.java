package net.vulkanmod.render.blockentity;

import it.unimi.dsi.fastutil.objects.Reference2BooleanOpenHashMap;
import net.minecraft.client.renderer.culling.Frustum;
import net.minecraft.core.BlockPos;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.resources.Identifier;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.entity.BlockEntityType;
import net.vulkanmod.mixin.render.blockentity.FrustumAccessor;

/**
 * Per block entity frustum culling.
 * <p>
 * There is no render bounding box for block entities, so a conservative box is used: the block plus one block on
 * every side, and two extra blocks above (standing banners reach ~2.6 blocks up). Only vanilla block entity types
 * are culled; modded ones may render arbitrarily large geometry from a single block, so they are left to vanilla's
 * per-section culling. Block entities that render off screen (beacons etc.) never reach this path.
 */
public final class BlockEntityCulling {
    private static final double MARGIN = 1.0;
    private static final double EXTRA_TOP = 2.0;

    private static final Reference2BooleanOpenHashMap<BlockEntityType<?>> CULLABLE_TYPES = new Reference2BooleanOpenHashMap<>();

    private BlockEntityCulling() {
    }

    public static boolean isOutsideFrustum(Frustum frustum, BlockEntity blockEntity) {
        if (frustum == null || !isCullable(blockEntity.getType())) {
            return false;
        }

        BlockPos pos = blockEntity.getBlockPos();
        int result = ((FrustumAccessor) frustum).invokeCubeInFrustum(
                pos.getX() - MARGIN, pos.getY() - MARGIN, pos.getZ() - MARGIN,
                pos.getX() + 1 + MARGIN, pos.getY() + 1 + MARGIN + EXTRA_TOP, pos.getZ() + 1 + MARGIN);
        return result != -2 && result != -1;
    }

    private static boolean isCullable(BlockEntityType<?> type) {
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
