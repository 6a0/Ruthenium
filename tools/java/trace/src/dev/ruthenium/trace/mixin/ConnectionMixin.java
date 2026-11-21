package dev.ruthenium.trace.mixin;

import dev.ruthenium.trace.Recorder;
import io.netty.channel.ChannelFutureListener;
import io.netty.channel.ChannelHandlerContext;
import net.minecraft.network.Connection;
import net.minecraft.network.PacketListener;
import net.minecraft.network.ProtocolInfo;
import net.minecraft.network.protocol.Packet;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/**
 * Outbound packets as the game hands them to the connection, inbound packets as they arrive, and
 * the protocol in effect (needed to encode packets: singleplayer's in-memory connection never
 * encodes them itself).
 */
@Mixin(Connection.class)
public abstract class ConnectionMixin {
    @Inject(method = "setupOutboundProtocol", at = @At("HEAD"))
    private void ruthenium$outbound(ProtocolInfo<?> protocol, CallbackInfo ci) {
        Recorder.protocol(protocol);
    }

    @Inject(method = "setupInboundProtocol", at = @At("HEAD"))
    private <T extends PacketListener> void ruthenium$inbound(ProtocolInfo<T> protocol, T listener, CallbackInfo ci) {
        Recorder.protocol(protocol);
    }

    @Inject(method = "send(Lnet/minecraft/network/protocol/Packet;Lio/netty/channel/ChannelFutureListener;Z)V", at = @At("HEAD"))
    private void ruthenium$send(Packet<?> packet, ChannelFutureListener listener, boolean flush, CallbackInfo ci) {
        Recorder.sent((Connection) (Object) this, packet);
    }

    @Inject(method = "channelRead0", at = @At("HEAD"))
    private void ruthenium$receive(ChannelHandlerContext ctx, Packet<?> packet, CallbackInfo ci) {
        Recorder.received((Connection) (Object) this, packet);
    }
}
