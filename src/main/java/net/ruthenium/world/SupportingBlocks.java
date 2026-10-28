package net.ruthenium.world;

import net.minecraft.core.BlockPos;
import net.minecraft.util.Mth;
import net.minecraft.world.level.BlockGetter;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockBehaviour;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.chunk.LevelChunk;
import net.minecraft.world.level.chunk.LevelChunkSection;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.Vec3;
import net.minecraft.world.phys.shapes.CollisionContext;
import net.ruthenium.interfaces.SectionChangeStamp;

import java.util.Arrays;
import java.util.Optional;

/**
 * Support for keeping an entity's {@code findSupportingBlock} answer while nothing it depends on changed.
 * <p>
 * Entities resting on the ground look for their supporting block every time they move down onto it (experience orbs
 * every tick, dropped items every fourth): {@code BlockCollisions} over a thin slab under them, 3x3x3 or more block
 * reads. The answer depends on the slab, the entity's position, and the collision shapes of the blocks
 * {@code BlockCollisions} visits for the entity's collision context. It is kept only when every block in that
 * region has a shape that is neither dynamic ({@link Block#hasDynamicShape}) nor computed from the collision
 * context (the block class keeps {@code BlockBehaviour.getCollisionShape}), and only while the chunk sections of
 * the region are the same objects with the same change stamp ({@link SectionChangeStamp}: bumped by every block
 * change of a client chunk and by chunk data read into it). Client levels only.
 */
public final class SupportingBlocks {
    /** Most chunk sections a kept region may span (2 per axis) */
    public static final int MAX_SECTIONS = 8;

    private static final ClassValue<Boolean> CONTEXT_FREE = new ClassValue<>() {
        @Override
        protected Boolean computeValue(Class<?> type) {
            for (Class<?> c = type; c != null && c != BlockBehaviour.class; c = c.getSuperclass()) {
                try {
                    c.getDeclaredMethod("getCollisionShape", BlockState.class, BlockGetter.class, BlockPos.class, CollisionContext.class);
                    return false;
                } catch (NoSuchMethodException e) {
                    // not declared here
                } catch (RuntimeException | LinkageError e) {
                    return false;
                }
            }
            return true;
        }
    };

    private SupportingBlocks() {
    }

    /** An entity's kept answer and what it depended on */
    public static final class Kept {
        private Level level;
        private double minX, minY, minZ, maxX, maxY, maxZ;
        private double x, y, z;
        public Optional<BlockPos> result;
        private final LevelChunkSection[] sections = new LevelChunkSection[MAX_SECTIONS];
        private final int[] stamps = new int[MAX_SECTIONS];
        private int sectionCount;

        public boolean matches(Level level, AABB box, Vec3 position) {
            return this.result != null && this.level == level
                    && this.minX == box.minX && this.minY == box.minY && this.minZ == box.minZ
                    && this.maxX == box.maxX && this.maxY == box.maxY && this.maxZ == box.maxZ
                    && this.x == position.x && this.y == position.y && this.z == position.z
                    && unchanged(level, box, this.sections, this.stamps, this.sectionCount);
        }

        public void store(Level level, AABB box, Vec3 position, Optional<BlockPos> result) {
            int count = capture(level, box, this.sections, this.stamps);
            if (count < 0) {
                this.result = null;
                this.level = null;
                Arrays.fill(this.sections, null);
                return;
            }
            this.level = level;
            this.minX = box.minX;
            this.minY = box.minY;
            this.minZ = box.minZ;
            this.maxX = box.maxX;
            this.maxY = box.maxY;
            this.maxZ = box.maxZ;
            this.x = position.x;
            this.y = position.y;
            this.z = position.z;
            this.result = result;
            this.sectionCount = count;
        }
    }

    /**
     * Records the chunk sections of the region {@code BlockCollisions} visits for this box (the box's blocks and one
     * more on each side) and their change stamps; returns their count, or -1 when the region is too large or holds a
     * block whose collision shape is dynamic or depends on the collision context
     */
    public static int capture(Level level, AABB box, LevelChunkSection[] sections, int[] stamps) {
        int x0 = Mth.floor(box.minX - 1.0E-7) - 1, x1 = Mth.floor(box.maxX + 1.0E-7) + 1;
        int y0 = Mth.floor(box.minY - 1.0E-7) - 1, y1 = Mth.floor(box.maxY + 1.0E-7) + 1;
        int z0 = Mth.floor(box.minZ - 1.0E-7) - 1, z1 = Mth.floor(box.maxZ + 1.0E-7) + 1;
        if ((long) (x1 - x0 + 1) * (y1 - y0 + 1) * (z1 - z0 + 1) > 64) {
            return -1;
        }
        int n = 0;
        for (int cx = x0 >> 4; cx <= x1 >> 4; cx++) {
            for (int cz = z0 >> 4; cz <= z1 >> 4; cz++) {
                LevelChunk chunk = level.getChunkSource().getChunkNow(cx, cz);
                for (int sy = y0 >> 4; sy <= y1 >> 4; sy++) {
                    if (n == MAX_SECTIONS) {
                        return -1;
                    }
                    LevelChunkSection section = section(chunk, sy);
                    sections[n] = section;
                    stamps[n] = section != null ? ((SectionChangeStamp) section).ruthenium$changes() : 0;
                    n++;
                }
            }
        }
        BlockPos.MutableBlockPos pos = new BlockPos.MutableBlockPos();
        for (int x = x0; x <= x1; x++) {
            for (int y = y0; y <= y1; y++) {
                for (int z = z0; z <= z1; z++) {
                    Block block = level.getBlockState(pos.set(x, y, z)).getBlock();
                    if (block.hasDynamicShape() || !CONTEXT_FREE.get(block.getClass())) {
                        return -1;
                    }
                }
            }
        }
        return n;
    }

    /** Whether the region of this box (the same box as at {@link #capture}) still has the recorded sections and stamps */
    public static boolean unchanged(Level level, AABB box, LevelChunkSection[] sections, int[] stamps, int count) {
        int x0 = Mth.floor(box.minX - 1.0E-7) - 1, x1 = Mth.floor(box.maxX + 1.0E-7) + 1;
        int y0 = Mth.floor(box.minY - 1.0E-7) - 1, y1 = Mth.floor(box.maxY + 1.0E-7) + 1;
        int z0 = Mth.floor(box.minZ - 1.0E-7) - 1, z1 = Mth.floor(box.maxZ + 1.0E-7) + 1;
        int n = 0;
        for (int cx = x0 >> 4; cx <= x1 >> 4; cx++) {
            for (int cz = z0 >> 4; cz <= z1 >> 4; cz++) {
                LevelChunk chunk = level.getChunkSource().getChunkNow(cx, cz);
                for (int sy = y0 >> 4; sy <= y1 >> 4; sy++) {
                    if (n == count) {
                        return false;
                    }
                    LevelChunkSection section = section(chunk, sy);
                    if (section != sections[n] || section != null && ((SectionChangeStamp) section).ruthenium$changes() != stamps[n]) {
                        return false;
                    }
                    n++;
                }
            }
        }
        return n == count;
    }

    private static LevelChunkSection section(LevelChunk chunk, int sectionY) {
        if (chunk == null) {
            return null;
        }
        int index = chunk.getSectionIndexFromSectionY(sectionY);
        LevelChunkSection[] sections = chunk.getSections();
        return index >= 0 && index < sections.length ? sections[index] : null;
    }
}
