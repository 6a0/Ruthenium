package dev.ruthenium.trace.mixin;

import dev.ruthenium.trace.Recorder;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.multiplayer.prediction.BlockStatePredictionHandler;
import net.minecraft.core.BlockPos;
import net.minecraft.world.level.block.state.BlockState;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfoReturnable;

/** Every block the client level sets, as the call is made, and whether an action is predicted. */
@Mixin(ClientLevel.class)
public abstract class ClientLevelMixin {
    @Shadow
    @Final
    private BlockStatePredictionHandler blockStatePredictionHandler;

    @Inject(method = "setBlock(Lnet/minecraft/core/BlockPos;Lnet/minecraft/world/level/block/state/BlockState;II)Z", at = @At("HEAD"))
    private void ruthenium$setBlock(BlockPos pos, BlockState state, int flags, int limit, CallbackInfoReturnable<Boolean> cir) {
        Recorder.blockSet(pos, state, flags, limit, this.blockStatePredictionHandler.isPredicting());
    }
}
