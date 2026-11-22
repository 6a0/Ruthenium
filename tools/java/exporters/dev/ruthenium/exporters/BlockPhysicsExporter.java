/*
 * Original Ruthenium code, GPL-3.0-or-later.
 */
package dev.ruthenium.exporters;

import java.io.IOException;
import java.lang.reflect.Field;
import java.lang.reflect.Method;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

import it.unimi.dsi.fastutil.doubles.DoubleList;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.level.EmptyBlockGetter;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.IceBlock;
import net.minecraft.world.level.block.LiquidBlock;
import net.minecraft.world.level.block.PowderSnowBlock;
import net.minecraft.world.level.block.ScaffoldingBlock;
import net.minecraft.world.level.block.piston.MovingPistonBlock;
import net.minecraft.world.level.block.state.BlockBehaviour;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.material.FlowingFluid;
import net.minecraft.world.level.material.Fluid;
import net.minecraft.world.level.material.FluidState;
import net.minecraft.world.level.material.Fluids;
import net.minecraft.world.phys.Vec3;
import net.minecraft.world.phys.shapes.CollisionContext;
import net.minecraft.world.phys.shapes.DiscreteVoxelShape;
import net.minecraft.world.phys.shapes.OffsetDoubleList;
import net.minecraft.world.phys.shapes.VoxelShape;

/**
 * What movement needs from every block state: its collision shape, exactly as the
 * game's collision code sees it, the block's friction and speed and jump factors, and
 * the state's fluid and sturdy faces.
 *
 * <p>Format {@code block_physics} v4, after the header:
 * <ul>
 *   <li>u32 shape count, then per shape, for x, y and z: u32 n and n f64 coordinates;
 *       then the (nx-1)(ny-1)(nz-1) voxels as bits, x outermost and z innermost, packed
 *       eight to a byte, lowest bit first. Shape 0 is the empty shape.</li>
 *   <li>u32 state count, then per state in id order: u32 shape, u8 flags (1: large
 *       collision shape, 2: dynamic shape, 4: shape depends on who collides, 8: offset
 *       by position, 16: suffocating, 32: an {@code IceBlock}), f32 friction, f32 speed
 *       factor, f32 jump factor, f32 bounce restitution, u8 fluid (bits 0-2: none, water,
 *       flowing water, lava, flowing lava; bit 3: falling; bits 4-7: amount), u8 sturdy
 *       faces ({@code isFaceSturdy} at the origin, one bit per {@code Direction} in
 *       ordinal order: down, up, north, south, west, east); with flag 8, also u8 offset
 *       type (1 XZ, 2 XYZ), f32 max horizontal and f32 max vertical offset; then u32
 *       outline shape and u32 interaction shape (in the same table) and u8 interaction (1: the block's class
 *       overrides {@code useWithoutItem}, 2: it overrides {@code useItemOn}).</li>
 * </ul>
 *
 * <p>The shape is the collision shape at the origin with an empty collision context, the
 * outline shape ({@code getShape}, what the crosshair picks) likewise.
 * For a block offset by position, its coordinates are those before the offset (the
 * game moves the base shape by the offset, then by the block's position).
 */
final class BlockPhysicsExporter {
    static final String FILE = "block_physics.bin";

    private static final int LARGE = 1;
    private static final int DYNAMIC = 2;
    private static final int CONTEXT = 4;
    private static final int OFFSET = 8;
    private static final int SUFFOCATING = 16;
    private static final int ICE = 32;

    private BlockPhysicsExporter() {}

    static void export(Path file) throws Exception {
        Field discrete = VoxelShape.class.getDeclaredField("shape");
        discrete.setAccessible(true);
        Field delegate = OffsetDoubleList.class.getDeclaredField("delegate");
        delegate.setAccessible(true);
        Method maxH = BlockBehaviour.class.getDeclaredMethod("getMaxHorizontalOffset");
        maxH.setAccessible(true);
        Method maxV = BlockBehaviour.class.getDeclaredMethod("getMaxVerticalOffset");
        maxV.setAccessible(true);
        Field bounce = BlockBehaviour.class.getDeclaredField("bounceRestitution");
        bounce.setAccessible(true);

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
            VoxelShape shape = state.getCollisionShape(EmptyBlockGetter.INSTANCE, BlockPos.ZERO, CollisionContext.empty());
            boolean offset = state.hasOffsetFunction();
            int id = intern(shape, discrete, delegate, offset, shapes, shapeIds);
            VoxelShape outlineShape = state.getShape(EmptyBlockGetter.INSTANCE, BlockPos.ZERO, CollisionContext.empty());
            int outline = intern(outlineShape, discrete, delegate, offset, shapes, shapeIds);
            VoxelShape interactionShape = state.getInteractionShape(EmptyBlockGetter.INSTANCE, BlockPos.ZERO);
            int interactionId = intern(interactionShape, discrete, delegate, offset, shapes, shapeIds);
            int flags = 0;
            if (state.hasLargeCollisionShape()) flags |= LARGE;
            if (block.hasDynamicShape()) flags |= DYNAMIC;
            if (block instanceof LiquidBlock || block instanceof PowderSnowBlock
                    || block instanceof ScaffoldingBlock || block instanceof MovingPistonBlock) {
                flags |= CONTEXT;
            }
            if (offset) flags |= OFFSET;
            if (state.isSuffocating(EmptyBlockGetter.INSTANCE, BlockPos.ZERO)) flags |= SUFFOCATING;
            if (block instanceof IceBlock) flags |= ICE;
            states.u32(id);
            states.u8(flags);
            states.f32(block.getFriction());
            states.f32(block.getSpeedFactor());
            states.f32(block.getJumpFactor());
            states.f32(bounce.getFloat(block));
            states.u8(fluid(state.getFluidState()));
            int sturdy = 0;
            for (Direction d : Direction.values()) {
                if (state.isFaceSturdy(EmptyBlockGetter.INSTANCE, BlockPos.ZERO, d)) sturdy |= 1 << d.ordinal();
            }
            states.u8(sturdy);
            if (offset) {
                boolean vertical = false;
                for (int x = 0; x < 16 && !vertical; x++) {
                    for (int z = 0; z < 16 && !vertical; z++) {
                        Vec3 o = state.getOffset(new BlockPos(x, 0, z));
                        vertical = o.y != 0.0;
                    }
                }
                states.u8(vertical ? 2 : 1);
                states.f32((Float) maxH.invoke(block));
                states.f32((Float) maxV.invoke(block));
            }
            states.u32(outline);
            states.u32(interactionId);
            states.u8(interaction(block.getClass()));
        }

        // The empty shape first, so 0 means "no collision".
        ExportWriter w = new ExportWriter("block_physics", 4);
        w.u32(shapes.size());
        for (byte[] s : shapes) {
            w.bytes(s);
        }
        w.u32(count);
        w.bytes(states.payload());
        w.writeTo(file);
    }

    /** A shape's index in the table, added if it is new. */
    private static int intern(VoxelShape shape, Field discrete, Field delegate, boolean offset,
            List<byte[]> shapes, Map<String, Integer> shapeIds) throws IllegalAccessException {
        byte[] encoded = encode(shape, (DiscreteVoxelShape) discrete.get(shape), delegate, offset);
        String key = java.util.Base64.getEncoder().encodeToString(encoded);
        Integer id = shapeIds.get(key);
        if (id == null) {
            id = shapes.size();
            shapes.add(encoded);
            shapeIds.put(key, id);
        }
        return id;
    }

    /** Whether a block class (or a superclass below {@code BlockBehaviour}) declares the use methods. */
    private static int interaction(Class<?> c) {
        int flags = 0;
        for (; c != null && c != BlockBehaviour.class; c = c.getSuperclass()) {
            for (Method m : c.getDeclaredMethods()) {
                if (m.isSynthetic() || m.isBridge()) continue;
                if (m.getName().equals("useWithoutItem")) flags |= 1;
                if (m.getName().equals("useItemOn")) flags |= 2;
            }
        }
        return flags;
    }

    /** A fluid state as one byte: its type, whether it is falling, and its amount. */
    private static int fluid(FluidState f) {
        if (f.isEmpty()) return 0;
        Fluid type = f.getType();
        int kind;
        if (type == Fluids.WATER) kind = 1;
        else if (type == Fluids.FLOWING_WATER) kind = 2;
        else if (type == Fluids.LAVA) kind = 3;
        else if (type == Fluids.FLOWING_LAVA) kind = 4;
        else throw new IllegalStateException("unknown fluid " + type);
        boolean falling = f.hasProperty(FlowingFluid.FALLING) && f.getValue(FlowingFluid.FALLING);
        return kind | (falling ? 8 : 0) | f.getAmount() << 4;
    }

    /** A shape's coordinates and voxels, in the file's layout. */
    private static byte[] encode(VoxelShape shape, DiscreteVoxelShape d, Field delegate, boolean offset)
            throws IllegalAccessException {
        ExportWriter w = new ExportWriter(null, 0);
        for (Direction.Axis axis : Direction.Axis.values()) {
            DoubleList coords = shape.getCoords(axis);
            if (offset && coords instanceof OffsetDoubleList) {
                coords = (DoubleList) delegate.get(coords);
            }
            w.u32(coords.size());
            for (int i = 0; i < coords.size(); i++) {
                w.f64(coords.getDouble(i));
            }
        }
        int nx = d.getXSize();
        int ny = d.getYSize();
        int nz = d.getZSize();
        int bit = 0;
        int acc = 0;
        for (int x = 0; x < nx; x++) {
            for (int y = 0; y < ny; y++) {
                for (int z = 0; z < nz; z++) {
                    if (d.isFull(x, y, z)) acc |= 1 << bit;
                    if (++bit == 8) {
                        w.u8(acc);
                        bit = 0;
                        acc = 0;
                    }
                }
            }
        }
        if (bit != 0) w.u8(acc);
        return w.payload();
    }
}
