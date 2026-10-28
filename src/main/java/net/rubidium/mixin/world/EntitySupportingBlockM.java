package net.rubidium.mixin.world;

import net.minecraft.core.BlockPos;
import net.minecraft.world.entity.Entity;
import net.minecraft.world.level.Level;
import net.minecraft.world.phys.AABB;
import net.minecraft.world.phys.Vec3;
import net.rubidium.Initializer;
import net.rubidium.world.SupportingBlocks;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Redirect;

import java.util.Optional;

/**
 * Keeps the last {@code findSupportingBlock} answer of an entity on the client, see {@link SupportingBlocks}. The
 * state lives in a separate object created on first use, so entities that never search (most mobs on the client)
 * only carry one reference.
 */
@Mixin(Entity.class)
public abstract class EntitySupportingBlockM {
    @Shadow public abstract Vec3 position();

    @Unique private SupportingBlocks.Kept keptSupport;

    @Redirect(method = "checkSupportingBlock", at = @At(value = "INVOKE",
            target = "Lnet/minecraft/world/level/Level;findSupportingBlock(Lnet/minecraft/world/entity/Entity;Lnet/minecraft/world/phys/AABB;)Ljava/util/Optional;"))
    private Optional<BlockPos> keepSupportingBlock(Level level, Entity source, AABB box) {
        if (!Initializer.CONFIG.fastSupportingBlocks || !level.isClientSide()) {
            return level.findSupportingBlock(source, box);
        }
        Vec3 position = this.position();
        SupportingBlocks.Kept kept = this.keptSupport;
        if (kept != null && kept.matches(level, box, position)) {
            return kept.result;
        }
        Optional<BlockPos> result = level.findSupportingBlock(source, box);
        if (kept == null) {
            kept = this.keptSupport = new SupportingBlocks.Kept();
        }
        kept.store(level, box, position, result);
        return result;
    }
}
