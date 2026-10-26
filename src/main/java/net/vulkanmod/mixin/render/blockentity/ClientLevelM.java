package net.vulkanmod.mixin.render.blockentity;

import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.core.BlockPos;
import net.vulkanmod.render.blockentity.RetainedChests;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/** Breaking progress moves a chest out of (or back into) its section's retained geometry */
@Mixin(ClientLevel.class)
public class ClientLevelM {
    @Inject(method = "destroyBlockProgress", at = @At("HEAD"))
    private void markRetainedDirty(int id, BlockPos pos, int progress, CallbackInfo ci) {
        RetainedChests.markDirty(pos);
    }
}
