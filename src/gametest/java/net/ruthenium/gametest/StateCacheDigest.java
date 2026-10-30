package net.ruthenium.gametest;

import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.world.level.EmptyBlockGetter;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.SupportType;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.phys.shapes.VoxelShape;

import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.util.HexFormat;

/**
 * A digest of every block state's cached data (collision and occlusion shapes, sturdy faces, light, fluid, solid), to
 * check that building the caches differently (e.g. in parallel, a twenty-ninth session prototype) gives the same results:
 * the footprint scene logs it with {@code -Pbench.loadProbe=true} and writes each state's line next to the histograms,
 * and two launches with the change on and off must log the same digest.
 */
final class StateCacheDigest {
    private StateCacheDigest() {
    }

    /** The digest; with {@code dump} non-null, every state's line is also written there (for diffing two launches) */
    static String digest(java.nio.file.Path dump) {
        try {
            java.io.Writer out = dump == null ? null : java.nio.file.Files.newBufferedWriter(dump);
            MessageDigest sha = MessageDigest.getInstance("SHA-256");
            StringBuilder sb = new StringBuilder(256);
            int states = 0;
            for (BlockState state : Block.BLOCK_STATE_REGISTRY) {
                states++;
                sb.setLength(0);
                sb.append(Block.BLOCK_STATE_REGISTRY.getId(state)).append(state).append('|')
                        .append(state.isSolid()).append(state.isSolidRender()).append(state.propagatesSkylightDown())
                        .append(state.getLightDampening()).append(state.hasLargeCollisionShape())
                        .append(state.isRandomlyTicking())
                        // Fluid has no toString of its own (an identity hash would differ per launch)
                        .append(BuiltInRegistries.FLUID.getKey(state.getFluidState().getType())).append(state.getFluidState().getAmount())
                        .append(state.getFluidState().isSource())
                        .append(state.isCollisionShapeFullBlock(EmptyBlockGetter.INSTANCE, BlockPos.ZERO));
                shape(sb, state.getCollisionShape(EmptyBlockGetter.INSTANCE, BlockPos.ZERO));
                shape(sb, state.getOcclusionShape());
                for (Direction direction : Direction.values()) {
                    shape(sb, state.getFaceOcclusionShape(direction));
                    for (SupportType type : SupportType.values()) {
                        sb.append(state.isFaceSturdy(EmptyBlockGetter.INSTANCE, BlockPos.ZERO, direction, type) ? '1' : '0');
                    }
                }
                sha.update(sb.toString().getBytes(StandardCharsets.UTF_8));
                if (out != null) {
                    out.write(sb.toString());
                    out.write('\n');
                }
            }
            if (out != null) {
                out.close();
            }
            return HexFormat.of().formatHex(sha.digest()) + " over " + states + " states";
        } catch (Exception e) {
            return "failed: " + e;
        }
    }

    private static void shape(StringBuilder sb, VoxelShape shape) {
        sb.append('[');
        shape.forAllBoxes((x0, y0, z0, x1, y1, z1) -> sb.append(x0).append(',').append(y0).append(',').append(z0).append(',')
                .append(x1).append(',').append(y1).append(',').append(z1).append(';'));
        sb.append(']');
    }
}
