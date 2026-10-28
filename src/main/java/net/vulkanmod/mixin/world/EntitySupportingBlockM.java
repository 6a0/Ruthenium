package net.vulkanmod.mixin.world;

import net.minecraft.core.BlockPos;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.chunk.LevelChunkSection;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.Vec3;
import net.vulkanmod.Initializer;
import net.vulkanmod.world.SupportingBlocks;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Redirect;

import java.util.Optional;

/** Keeps the last {@code findSupportingBlock} answer of an entity on the client, see {@link SupportingBlocks} */
@Mixin(Entity.class)
public abstract class EntitySupportingBlockM {
    @Shadow public abstract Vec3 position();

    @Unique private Level supportLevel;
    @Unique private double supportMinX, supportMinY, supportMinZ, supportMaxX, supportMaxY, supportMaxZ;
    @Unique private double supportX, supportY, supportZ;
    @Unique private Optional<BlockPos> supportResult;
    @Unique private LevelChunkSection[] supportSections;
    @Unique private int[] supportStamps;
    @Unique private int supportSectionCount;

    @Redirect(method = "checkSupportingBlock", at = @At(value = "INVOKE",
            target = "Lnet/minecraft/world/level/Level;findSupportingBlock(Lnet/minecraft/world/entity/Entity;Lnet/minecraft/world/phys/AABB;)Ljava/util/Optional;"))
    private Optional<BlockPos> keepSupportingBlock(Level level, Entity source, AABB box) {
        if (!Initializer.CONFIG.fastSupportingBlocks || !level.isClientSide()) {
            return level.findSupportingBlock(source, box);
        }
        Vec3 position = this.position();
        if (this.supportResult != null && this.supportLevel == level
                && this.supportMinX == box.minX && this.supportMinY == box.minY && this.supportMinZ == box.minZ
                && this.supportMaxX == box.maxX && this.supportMaxY == box.maxY && this.supportMaxZ == box.maxZ
                && this.supportX == position.x && this.supportY == position.y && this.supportZ == position.z
                && SupportingBlocks.unchanged(level, box, this.supportSections, this.supportStamps, this.supportSectionCount)) {
            return this.supportResult;
        }
        Optional<BlockPos> result = level.findSupportingBlock(source, box);
        if (this.supportSections == null) {
            this.supportSections = new LevelChunkSection[SupportingBlocks.MAX_SECTIONS];
            this.supportStamps = new int[SupportingBlocks.MAX_SECTIONS];
        }
        int count = SupportingBlocks.capture(level, box, this.supportSections, this.supportStamps);
        if (count >= 0) {
            this.supportLevel = level;
            this.supportMinX = box.minX;
            this.supportMinY = box.minY;
            this.supportMinZ = box.minZ;
            this.supportMaxX = box.maxX;
            this.supportMaxY = box.maxY;
            this.supportMaxZ = box.maxZ;
            this.supportX = position.x;
            this.supportY = position.y;
            this.supportZ = position.z;
            this.supportResult = result;
            this.supportSectionCount = count;
        } else {
            this.supportResult = null;
            this.supportLevel = null;
            java.util.Arrays.fill(this.supportSections, null);
        }
        return result;
    }
}
