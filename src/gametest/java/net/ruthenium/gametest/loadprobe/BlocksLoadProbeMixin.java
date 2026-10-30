package net.ruthenium.gametest.loadprobe;

import net.minecraft.world.level.block.Blocks;
import net.ruthenium.gametest.LoadProbe;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/** How long the block state caches take to build in the blocks' static init (see {@link LoadProbe}) */
@Mixin(Blocks.class)
public class BlocksLoadProbeMixin {
    @Inject(method = "<clinit>", at = @At(value = "INVOKE", target = "Lnet/minecraft/world/level/block/state/SolidDebugger;runAndDump(Ljava/lang/Runnable;)V"))
    private static void ruthenium$before(CallbackInfo ci) {
        LoadProbe.mark("block state caches start");
    }

    @Inject(method = "<clinit>", at = @At(value = "INVOKE", target = "Lnet/minecraft/world/level/block/state/SolidDebugger;runAndDump(Ljava/lang/Runnable;)V", shift = At.Shift.AFTER))
    private static void ruthenium$after(CallbackInfo ci) {
        LoadProbe.mark("block state caches done");
    }
}
