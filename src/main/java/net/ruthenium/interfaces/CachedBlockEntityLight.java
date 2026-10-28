package net.ruthenium.interfaces;

/**
 * The light of a block entity (and that its block state is valid for it), cached by
 * {@link net.ruthenium.render.blockentity.InstancedModels} for as long as the stamp is current.
 */
public interface CachedBlockEntityLight {
    int ruthenium$getLightStamp();

    int ruthenium$getLight();

    void ruthenium$setLight(int stamp, int light);
}
