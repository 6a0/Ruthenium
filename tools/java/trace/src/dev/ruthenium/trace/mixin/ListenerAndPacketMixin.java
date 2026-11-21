package dev.ruthenium.trace.mixin;

import dev.ruthenium.trace.Recorder;
import net.minecraft.network.PacketListener;
import net.minecraft.network.protocol.Packet;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/** The point where a queued inbound packet is processed on the game thread. */
@Mixin(targets = "net.minecraft.network.PacketProcessor$ListenerAndPacket")
public abstract class ListenerAndPacketMixin {
    @Shadow
    @Final
    private PacketListener listener;

    @Shadow
    @Final
    private Packet<?> packet;

    @Inject(method = "handle", at = @At("HEAD"))
    private void ruthenium$handle(CallbackInfo ci) {
        Recorder.handled(this.listener, this.packet);
    }
}
