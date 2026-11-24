/*
 * Original Ruthenium code, GPL-3.0-or-later.
 */
package dev.ruthenium.exporters;

import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

import net.minecraft.client.color.block.BlockColors;
import net.minecraft.client.color.block.BlockTintSource;
import net.minecraft.client.renderer.BiomeColors;
import net.minecraft.client.renderer.block.BlockAndTintGetter;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.level.CardinalLighting;
import net.minecraft.world.level.ColorResolver;
import net.minecraft.world.level.EmptyBlockGetter;
import net.minecraft.world.level.GrassColor;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.HalfTransparentBlock;
import net.minecraft.world.level.block.IronBarsBlock;
import net.minecraft.world.level.block.LeavesBlock;
import net.minecraft.world.level.block.LiquidBlock;
import net.minecraft.world.level.block.MangroveRootsBlock;
import net.minecraft.world.level.block.PowderSnowBlock;
import net.minecraft.world.level.block.RenderShape;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.state.BlockBehaviour;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.lighting.LevelLightEngine;
import net.minecraft.world.level.material.FluidState;
import net.minecraft.world.level.material.Fluids;
import net.minecraft.world.phys.shapes.OffsetDoubleList;
import net.minecraft.world.phys.shapes.VoxelShape;

/**
 * What drawing a block state as terrain needs that its model files don't say: whether
 * it is drawn from a model at all, the shape each of its faces hides a neighbour's face
 * with, the rule by which it leaves out faces against particular neighbours, and the
 * colours its tinted faces are multiplied by.
 *
 * <p>Format {@code block_render} v2, after the header:
 * <ul>
 *   <li>u32 shape count, then the shapes as in {@code block_physics} (shape 0 is the
 *       empty shape).</li>
 *   <li>u32 state count, then per state in id order: u8 flags (1: drawn from its model,
 *       {@code RenderShape.MODEL}; 2: {@code emissiveRendering}; 4: a {@code
 *       LeavesBlock}; 8: {@code getShadeBrightness} is 0.2, where it is 1 otherwise: how
 *       much the state darkens the corners of faces beside it under smooth lighting);
 *       u8 the rule its class leaves faces out by ({@code skipRendering}:
 *       0 none, 1 against the same block, 2 leaves against leaves when leaves are drawn
 *       opaque, 3 bars, 4 mangrove roots above and below each other, 5 a liquid against
 *       the same fluid); six u32 face occlusion shapes, one per {@code Direction} in
 *       ordinal order ({@code getFaceOcclusionShape}), in the shape table; u8 tint layer
 *       count, then per layer: u8 1 if the colour outside a world is the grass colour
 *       map's default, u32 that colour otherwise; u8 what the colour in a world is (0 a
 *       constant, 1 the biomes' grass colour, 2 foliage, 3 dry foliage, 4 water), u32 the
 *       constant, and u8 (signed) how far above the block the biomes are read (-1: the
 *       upper half of a double plant reads at its lower half); then three u8 (signed):
 *       where, from the block, the position lies that seeds its choice of model
 *       variants ({@code getSeed}: the lower half of a door or a double plant, a bed's
 *       head).</li>
 * </ul>
 */
final class BlockRenderExporter {
    static final String FILE = "block_render.bin";

    private static final int MODEL = 1;
    private static final int EMISSIVE = 2;
    private static final int LEAVES = 4;
    private static final int DARK = 8;

    private static final int SKIP_NONE = 0;
    private static final int SKIP_SAME_BLOCK = 1;
    private static final int SKIP_LEAVES = 2;
    private static final int SKIP_BARS = 3;
    private static final int SKIP_MANGROVE_ROOTS = 4;
    private static final int SKIP_SAME_FLUID = 5;

    /** A colour no tint source gives: marks the grass colour map's default. */
    private static final int GRASS_DEFAULT = 0x00A1B2C3;
    private static final int PROBE_Y = 64;

    private BlockRenderExporter() {}

    /** A level that only says which biome colour was asked for, and where. */
    private static final class Probe implements BlockAndTintGetter {
        int kind;
        int dy;

        @Override
        public CardinalLighting cardinalLighting() {
            return CardinalLighting.DEFAULT;
        }

        @Override
        public LevelLightEngine getLightEngine() {
            return LevelLightEngine.EMPTY;
        }

        @Override
        public int getBlockTint(BlockPos pos, ColorResolver color) {
            if (color == BiomeColors.GRASS_COLOR_RESOLVER) kind = 1;
            else if (color == BiomeColors.FOLIAGE_COLOR_RESOLVER) kind = 2;
            else if (color == BiomeColors.DRY_FOLIAGE_COLOR_RESOLVER) kind = 3;
            else if (color == BiomeColors.WATER_COLOR_RESOLVER) kind = 4;
            else throw new IllegalStateException("an unknown biome colour was asked for");
            if (pos.getX() != 0 || pos.getZ() != 0) {
                throw new IllegalStateException("a tint is read beside its block, at " + pos);
            }
            dy = pos.getY() - PROBE_Y;
            return 0;
        }

        @Override
        public BlockEntity getBlockEntity(BlockPos pos) {
            return null;
        }

        @Override
        public BlockState getBlockState(BlockPos pos) {
            return Blocks.AIR.defaultBlockState();
        }

        @Override
        public FluidState getFluidState(BlockPos pos) {
            return Fluids.EMPTY.defaultFluidState();
        }

        @Override
        public int getHeight() {
            return 0;
        }

        @Override
        public int getMinY() {
            return 0;
        }
    }

    static void export(Path file) throws Exception {
        Field discrete = VoxelShape.class.getDeclaredField("shape");
        discrete.setAccessible(true);
        Field delegate = OffsetDoubleList.class.getDeclaredField("delegate");
        delegate.setAccessible(true);

        int[] grass = new int[65536];
        Arrays.fill(grass, GRASS_DEFAULT);
        GrassColor.init(grass);
        BlockColors colors = BlockColors.createDefault();
        Map<Class<?>, Integer> rules = new HashMap<>();

        List<byte[]> shapes = new ArrayList<>();
        Map<String, Integer> shapeIds = new HashMap<>();
        ExportWriter states = new ExportWriter(null, 0);
        int count = 0;
        for (BlockState state : Block.BLOCK_STATE_REGISTRY) {
            if (Block.BLOCK_STATE_REGISTRY.getId(state) != count) {
                throw new IllegalStateException("block states out of order at " + count);
            }
            count++;
            Block block = state.getBlock();
            int flags = 0;
            if (state.getRenderShape() == RenderShape.MODEL) flags |= MODEL;
            if (state.emissiveRendering()) flags |= EMISSIVE;
            if (block instanceof LeavesBlock) flags |= LEAVES;
            float shade = state.getShadeBrightness(EmptyBlockGetter.INSTANCE, BlockPos.ZERO);
            if (shade == 0.2F) {
                flags |= DARK;
            } else if (shade != 1.0F) {
                throw new IllegalStateException("shade brightness " + shade + " of " + state);
            }
            states.u8(flags);
            Integer rule = rules.get(block.getClass());
            if (rule == null) {
                rule = skipRule(block.getClass());
                rules.put(block.getClass(), rule);
            }
            states.u8(rule);
            for (Direction d : Direction.values()) {
                VoxelShape face = state.getFaceOcclusionShape(d);
                states.u32(BlockPhysicsExporter.intern(face, discrete, delegate, false, shapes, shapeIds));
            }
            List<BlockTintSource> layers = colors.getTintSources(state);
            if (layers.size() > 255) {
                throw new IllegalStateException(layers.size() + " tint layers on " + state);
            }
            states.u8(layers.size());
            for (BlockTintSource source : layers) {
                int hand = source.color(state);
                states.u8(hand == GRASS_DEFAULT ? 1 : 0);
                states.u32(hand == GRASS_DEFAULT ? 0 : hand);
                Probe probe = new Probe();
                int world = source.colorInWorld(state, probe, new BlockPos(0, PROBE_Y, 0));
                if (probe.kind == 0 && world == GRASS_DEFAULT) {
                    throw new IllegalStateException("the grass map's default colour in a world, on " + state);
                }
                states.u8(probe.kind);
                states.u32(probe.kind == 0 ? world : 0);
                states.u8(probe.dy);
            }
            for (int offset : seedOffset(state)) {
                states.u8(offset);
            }
        }

        ExportWriter w = new ExportWriter("block_render", 2);
        w.u32(shapes.size());
        for (byte[] s : shapes) {
            w.bytes(s);
        }
        w.u32(count);
        w.bytes(states.payload());
        w.writeTo(file);
    }

    /**
     * Which position's seed a state's seed is: the one block offset whose plain seed is
     * the state's at two positions.
     */
    private static int[] seedOffset(BlockState state) {
        BlockPos a = new BlockPos(1234, 71, -5678);
        BlockPos b = new BlockPos(-91, -40, 3);
        // Stone's seed is its position's.
        BlockState plain = Blocks.STONE.defaultBlockState();
        int[] found = null;
        for (int dx = -1; dx <= 1; dx++) {
            for (int dy = -1; dy <= 1; dy++) {
                for (int dz = -1; dz <= 1; dz++) {
                    if (state.getSeed(a) == plain.getSeed(a.offset(dx, dy, dz))
                            && state.getSeed(b) == plain.getSeed(b.offset(dx, dy, dz))) {
                        if (found != null) {
                            throw new IllegalStateException("two positions seed " + state);
                        }
                        found = new int[] {dx, dy, dz};
                    }
                }
            }
        }
        if (found == null) {
            throw new IllegalStateException("the seed of " + state + " is of no position beside it");
        }
        return found;
    }

    /**
     * The rule of the class that decides {@code skipRendering} for a block: the nearest
     * one to declare it. A class this exporter doesn't know stops the export, so a new
     * rule can't go unnoticed.
     */
    private static int skipRule(Class<?> block) {
        for (Class<?> c = block; c != null; c = c.getSuperclass()) {
            for (Method m : c.getDeclaredMethods()) {
                if (m.isSynthetic() || m.isBridge() || !m.getName().equals("skipRendering")) continue;
                if (c == BlockBehaviour.class) return SKIP_NONE;
                if (c == HalfTransparentBlock.class || c == PowderSnowBlock.class) return SKIP_SAME_BLOCK;
                if (c == LeavesBlock.class) return SKIP_LEAVES;
                if (c == IronBarsBlock.class) return SKIP_BARS;
                if (c == MangroveRootsBlock.class) return SKIP_MANGROVE_ROOTS;
                if (c == LiquidBlock.class) return SKIP_SAME_FLUID;
                throw new IllegalStateException("skipRendering of " + c.getName() + " is not known");
            }
        }
        throw new IllegalStateException("no skipRendering for " + block.getName());
    }
}
