package net.vulkanmod.mixin.render.blockentity;

import net.minecraft.client.multiplayer.ClientPacketListener;
import net.minecraft.network.protocol.game.ClientboundBlockEntityDataPacket;
import net.vulkanmod.render.blockentity.RetainedBlockEntities;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/**
 * Block entity data can change what a retained block entity looks like (pot sherds, skull profiles) without a block
 * change, and often arrives after the block itself
 */
@Mixin(ClientPacketListener.class)
public class BlockEntityDataPacketM {
    @Inject(method = "handleBlockEntityData", at = @At("RETURN"))
    private void markRetainedDirty(ClientboundBlockEntityDataPacket packet, CallbackInfo ci) {
        RetainedBlockEntities.markDirty(packet.getPos());
    }
}
