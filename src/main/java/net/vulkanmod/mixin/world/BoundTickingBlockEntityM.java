package net.vulkanmod.mixin.world;

import net.minecraft.core.BlockPos;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.entity.BlockEntityType;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.chunk.LevelChunk;
import net.vulkanmod.Initializer;
import net.vulkanmod.interfaces.BlockEntityTicker;
import net.vulkanmod.interfaces.IdleTicker;
import org.jspecify.annotations.Nullable;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Redirect;

/**
 * The ticker bound to a block entity. A new one is bound for every block state of the block entity: a successful
 * {@code LevelChunk.setBlockState} at its position either removes the block entity (its wrapper is rebound to a null
 * ticker) or binds a new ticker ({@code updateBlockEntityTicker}), and a chunk replaced by packet data gets new
 * block entities. So on the client, the block state that passed vanilla's per tick validity check (a chunk section
 * lookup and a set lookup, ~75 ns per enchanting table when ~5.6k of them tick) is kept for the ticker's lifetime.
 * The world border check before it still runs every tick.
 */
@Mixin(targets = "net.minecraft.world.level.chunk.LevelChunk$BoundTickingBlockEntity")
public class BoundTickingBlockEntityM implements IdleTicker, BlockEntityTicker {
    @Shadow @Final private BlockEntity blockEntity;

    /** The block state at the block entity's position, once valid for ticking */
    @Unique private @Nullable BlockState validState;

    @Override
    public boolean vulkanmod$isTickIdle() {
        return this.blockEntity instanceof IdleTicker idle && idle.vulkanmod$isTickIdle();
    }

    @Override
    public BlockEntity vulkanmod$getBlockEntity() {
        return this.blockEntity;
    }

    /** Redirects rather than wrappers: this runs for every ticking block entity every tick */
    @Redirect(method = "tick", at = @At(value = "INVOKE",
            target = "Lnet/minecraft/world/level/chunk/LevelChunk;getBlockState(Lnet/minecraft/core/BlockPos;)Lnet/minecraft/world/level/block/state/BlockState;"))
    private BlockState useValidState(LevelChunk chunk, BlockPos pos) {
        BlockState state = this.validState;
        if (state != null && Initializer.CONFIG.cacheTickerStates) {
            return state;
        }
        return chunk.getBlockState(pos);
    }

    @Redirect(method = "tick", at = @At(value = "INVOKE",
            target = "Lnet/minecraft/world/level/block/entity/BlockEntityType;isValid(Lnet/minecraft/world/level/block/state/BlockState;)Z"))
    private boolean keepValidState(BlockEntityType<?> type, BlockState state) {
        if (state == this.validState) {
            return true;
        }
        boolean valid = type.isValid(state);
        Level level = this.blockEntity.getLevel();
        if (valid && level != null && level.isClientSide()) {
            this.validState = state;
        }
        return valid;
    }
}
