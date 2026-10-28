package net.rubidium.interfaces;

/**
 * The light of a block entity (and that its block state is valid for it), cached by
 * {@link net.rubidium.render.blockentity.InstancedModels} for as long as the stamp is current.
 */
public interface CachedBlockEntityLight {
    int rubidium$getLightStamp();

    int rubidium$getLight();

    void rubidium$setLight(int stamp, int light);
}
