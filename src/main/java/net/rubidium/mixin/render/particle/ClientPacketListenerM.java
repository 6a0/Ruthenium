package net.rubidium.mixin.render.particle;

import com.llamalad7.mixinextras.injector.wrapoperation.Operation;
import com.llamalad7.mixinextras.injector.wrapoperation.WrapOperation;
import net.minecraft.client.Minecraft;
import net.minecraft.client.multiplayer.ClientPacketListener;
import net.minecraft.network.protocol.game.ClientboundLevelParticlesPacket;
import net.rubidium.interfaces.ParticleSpawnLimiter;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;

/**
 * Stops spawning the particles of a particle packet once the particle engine starts dropping them (see
 * {@link ParticleEngineM}): all particles of a packet share a type, so the rest would be dropped too.
 * Vanilla already ends the loop when {@code tryAddParticle} returns false.
 */
@Mixin(ClientPacketListener.class)
public class ClientPacketListenerM {

    @WrapOperation(method = "handleParticleEvent", at = @At(value = "INVOKE",
            target = "Lnet/minecraft/client/multiplayer/ClientPacketListener;tryAddParticle(Lnet/minecraft/network/protocol/game/ClientboundLevelParticlesPacket;DDDDDD)Z"))
    private boolean stopWhenSaturated(ClientPacketListener instance, ClientboundLevelParticlesPacket packet,
                                      double x, double y, double z, double xa, double ya, double za,
                                      Operation<Boolean> original) {
        ParticleSpawnLimiter limiter = (ParticleSpawnLimiter) Minecraft.getInstance().particleEngine;
        limiter.resetLastAddRejected();

        boolean added = original.call(instance, packet, x, y, z, xa, ya, za);
        return added && !limiter.wasLastAddRejected();
    }
}
