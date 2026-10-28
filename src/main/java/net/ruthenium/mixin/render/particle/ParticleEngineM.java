package net.ruthenium.mixin.render.particle;

import it.unimi.dsi.fastutil.objects.Object2IntOpenHashMap;
import net.minecraft.client.particle.Particle;
import net.minecraft.client.particle.ParticleEngine;
import net.minecraft.client.particle.ParticleGroup;
import net.minecraft.client.particle.ParticleRenderType;
import net.ruthenium.Initializer;
import net.ruthenium.interfaces.ParticleSpawnLimiter;
import org.spongepowered.asm.mixin.Final;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.Unique;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

import java.util.Map;

/**
 * Drops particles at spawn time that the group would reject when the queue is drained on the next tick.
 * <p>
 * A group holds at most 16384 particles, so once it is full every queued particle is discarded anyway: those are
 * rejected exactly. Below that, vanilla accepts queued particles with a falling probability past 12288, so the
 * number of candidates queued per group per tick is capped at {@link #MAX_PENDING_PER_GROUP}; with that many
 * candidates a group fills to within ~1% of what an unbounded queue (e.g. a 500k /particle command) would reach.
 */
@Mixin(ParticleEngine.class)
public class ParticleEngineM implements ParticleSpawnLimiter {
    @Unique private static final int MAX_PARTICLES_PER_GROUP = 16384;
    @Unique private static final int MAX_PENDING_PER_GROUP = MAX_PARTICLES_PER_GROUP * 8;

    @Shadow @Final private Map<ParticleRenderType, ParticleGroup<?>> particles;

    @Unique private final Object2IntOpenHashMap<ParticleRenderType> pendingCounts = new Object2IntOpenHashMap<>();
    @Unique private boolean lastAddRejected;

    @Inject(method = "add", at = @At("HEAD"), cancellable = true)
    private void limitQueuedParticles(Particle particle, CallbackInfo ci) {
        if (!Initializer.CONFIG.particleSpawnLimit) {
            this.lastAddRejected = false;
            return;
        }

        ParticleRenderType group = particle.getGroup();
        ParticleGroup<?> existing = this.particles.get(group);
        int pending = this.pendingCounts.getInt(group);

        if ((existing != null && existing.size() >= MAX_PARTICLES_PER_GROUP) || pending >= MAX_PENDING_PER_GROUP) {
            this.lastAddRejected = true;
            ci.cancel();
            return;
        }

        this.pendingCounts.addTo(group, 1);
        this.lastAddRejected = false;
    }

    @Inject(method = {"tick", "clearParticles"}, at = @At("RETURN"))
    private void resetPendingCounts(CallbackInfo ci) {
        this.pendingCounts.clear();
    }

    @Override
    public void resetLastAddRejected() {
        this.lastAddRejected = false;
    }

    @Override
    public boolean wasLastAddRejected() {
        return this.lastAddRejected;
    }
}
