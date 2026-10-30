package net.ruthenium.gametest.loadprobe;

import it.unimi.dsi.fastutil.longs.LongSet;
import net.minecraft.server.network.PlayerChunkSender;
import net.ruthenium.gametest.LoadProbe;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/** The server's chunk send rate as the client acknowledges batches, every 10th batch (see {@link LoadProbe}) */
@Mixin(PlayerChunkSender.class)
public class PlayerChunkSenderLoadProbeMixin {
    @Shadow @Final private LongSet pendingChunks;
    @Shadow private float desiredChunksPerTick;
    @Shadow private int unacknowledgedBatches;

    private int ruthenium$batches;

    @Inject(method = "onChunkBatchReceivedByClient", at = @At("TAIL"))
    private void ruthenium$acknowledged(float desired, CallbackInfo ci) {
        if (this.ruthenium$batches++ % 10 == 0) {
            LoadProbe.mark("chunk batch " + this.ruthenium$batches + " acknowledged: desired " + this.desiredChunksPerTick
                    + " chunks per tick, " + this.pendingChunks.size() + " chunks pending, " + this.unacknowledgedBatches + " batches unacknowledged");
        }
    }
}
