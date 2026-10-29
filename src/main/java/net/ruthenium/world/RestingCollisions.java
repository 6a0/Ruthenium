package net.ruthenium.world;

import net.minecraft.world.entity.Entity;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.chunk.LevelChunkSection;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.Vec3;
import org.jspecify.annotations.Nullable;

/**
 * Keeps an entity's last block collision answer ({@code Entity.collideBoundingBox} in {@code Entity.collide}) while
 * nothing it depends on changed.
 * <p>
 * Dropped items and experience orbs resting on the ground move down onto it again and again (items every fourth tick,
 * orbs every tick), each time a {@code BlockCollisions} sweep over the box stretched by the movement (36 or more block
 * reads, a shape per colliding block) and {@code Shapes.collide}. Without entity colliders and with the world border
 * not close, the answer depends only on the box, the movement and the collision shapes of the blocks the sweep
 * visits, so it is kept under the conditions of {@link SupportingBlocks}: no block in the region with a dynamic or
 * context dependent shape, and the same chunk sections with the same change stamps. The region is captured only when
 * the same box and movement come twice in a row, so moving entities only pay for comparing them. Client levels only.
 */
public final class RestingCollisions {
    private RestingCollisions() {
    }

    /** An entity's last answer and what it depended on */
    public static final class Kept {
        private @Nullable Level level;
        private double minX, minY, minZ, maxX, maxY, maxZ;
        private double dx, dy, dz;
        private @Nullable Vec3 result;
        /** The box stretched by the movement, while the answer is kept */
        private @Nullable AABB region;
        private final LevelChunkSection[] sections = new LevelChunkSection[SupportingBlocks.MAX_SECTIONS];
        private final int[] stamps = new int[SupportingBlocks.MAX_SECTIONS];
        private int sectionCount;

        /** The kept answer for this box and movement, or null */
        public @Nullable Vec3 find(Level level, Entity source, AABB box, Vec3 movement) {
            AABB region = this.region;
            if (region == null || !this.isSame(level, box, movement)
                    || !SupportingBlocks.unchanged(level, region, this.sections, this.stamps, this.sectionCount)
                    || level.getWorldBorder().isInsideCloseToBorder(source, region)) {
                return null;
            }
            return this.result;
        }

        /** Records an answer computed by vanilla; keeps it if the same box and movement came last time too */
        public void store(Level level, Entity source, AABB box, Vec3 movement, Vec3 result) {
            if (!this.isSame(level, box, movement)) {
                this.level = level;
                this.minX = box.minX;
                this.minY = box.minY;
                this.minZ = box.minZ;
                this.maxX = box.maxX;
                this.maxY = box.maxY;
                this.maxZ = box.maxZ;
                this.dx = movement.x;
                this.dy = movement.y;
                this.dz = movement.z;
                this.forget();
                return;
            }
            // The region Entity.collideBoundingBox sweeps
            AABB region = box.expandTowards(movement);
            int count = level.getWorldBorder().isInsideCloseToBorder(source, region) ? -1
                    : SupportingBlocks.capture(level, region, this.sections, this.stamps);
            if (count < 0) {
                this.forget();
                return;
            }
            this.region = region;
            this.result = result;
            this.sectionCount = count;
        }

        private void forget() {
            this.region = null;
            this.result = null;
            if (this.sectionCount > 0) {
                java.util.Arrays.fill(this.sections, null);
                this.sectionCount = 0;
            }
        }

        private boolean isSame(Level level, AABB box, Vec3 movement) {
            return this.level == level && this.dx == movement.x && this.dy == movement.y && this.dz == movement.z
                    && this.minX == box.minX && this.minY == box.minY && this.minZ == box.minZ
                    && this.maxX == box.maxX && this.maxY == box.maxY && this.maxZ == box.maxZ;
        }
    }
}
